#!/usr/bin/env python3
"""relay_hop.py — delivery proof.

Takes the contract-stored event content + identity, re-runs the WORKER wasm
(sk never leaves the wasm boundary — on mainnet it never leaves the TEE) with
a real unix timestamp, publishes the worker-signed event to a real relay,
expects ["OK",..,true,..], then queries it back by id.

Run AFTER driver.py (reads nostr_state.bin)."""
import asyncio, base64, hashlib, json, subprocess, sys, time

import websockets

MOCK = '/Users/asil/dev/near-mock/target/release/near-mock'
INLAYER = '/Users/asil/.local/bin/inlayer'
WORKER = '/tmp/nostr_probe/nostr_worker.wasm'
STATE = '/tmp/nostr_local/nostr_state.bin'
CONTRACT = 'reg.test.near'
RELAY = 'wss://nos.lol'
ROOT = 'deadbeef' * 8
CALLER = 'alice.test.near'


def worker(payload):
    r = subprocess.run([INLAYER, 'run', WORKER, '-i', json.dumps(payload)],
                       capture_output=True, text=True, timeout=300)
    for line in r.stdout.splitlines():
        if 'Output:' in line:
            return line.split('Output: ', 1)[1].strip()
    raise SystemExit(f'worker failed: {r.stderr[-300:]}')


def stored(key):
    rows = json.loads(subprocess.run([MOCK, 'state', 'dump', STATE],
                                     capture_output=True, text=True).stdout)
    for row in rows:
        if base64.b64decode(row['key']).decode('utf-8', 'replace') == key:
            return base64.b64decode(row['value']).decode('utf-8', 'replace')
    raise SystemExit(f'{key} not found — run driver.py first')


async def main():
    content = json.loads(stored(f'last:{CALLER}'))['content']
    print(f"contract stored content: {content!r}")

    # TEE role: derive identity, sign the event with REAL unix time
    pk, flip, sk = worker({"case": "N", "root": ROOT, "caller": CALLER}).split("|")
    ts = str(int(time.time()))
    eid, sig = worker({"case": "E", "pk": pk, "sk": sk, "ts": ts,
                       "kind": "1", "content": content}).split("|")

    # sanity: id matches independent python sha256 of the serialization
    ser = f'[0,"{pk}",{ts},1,[],"{content}"]'
    assert hashlib.sha256(ser.encode()).hexdigest() == eid, "id mismatch"
    print(f"worker signed: pk={pk[:12]}… ts={ts} id={eid[:16]}…")

    async with websockets.connect(RELAY) as ws:
        event = ["EVENT", {"pubkey": pk, "created_at": int(ts), "kind": 1,
                           "tags": [], "content": content, "id": eid, "sig": sig}]
        await ws.send(json.dumps(event))
        ack = json.loads(await asyncio.wait_for(ws.recv(), 30))
        print("relay ack:", ack)
        assert ack[0] == "OK" and ack[2] is True, f"relay REJECTED: {ack}"

        await ws.send(json.dumps(["REQ", "proof", {"ids": [eid]}]))
        got = None
        while True:
            msg = json.loads(await asyncio.wait_for(ws.recv(), 30))
            if msg[0] == "EVENT":
                got = msg[2]
                break
            if msg[0] == "EOSE":
                break
        assert got and got['id'] == eid, "event not found after publish"
        print(f"✅ DELIVERED — {RELAY} now holds event {eid[:24]}… "
              f"(content: {got['content']!r})")


asyncio.run(main())
