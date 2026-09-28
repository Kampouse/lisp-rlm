#!/usr/bin/env python3
"""driver.py — local end-to-end: contract post → yield → worker signs → resume → on-chain verify.

Plays the role an OutLayer TEE worker plays on mainnet. The driver is UNTRUSTED
by design: caller/content/pk are frozen in the yield spec at post() time; the
contract re-verifies id + schnorr sig on-chain (host k256) before storing.
"""
import json, os, subprocess, sys, hashlib

MOCK = os.path.expanduser('~/dev/near-mock/target/release/near-mock')
INLAYER = os.path.expanduser('~/.local/bin/inlayer')
WORKER = '/tmp/nostr_probe/nostr_worker.wasm'
REG = '/tmp/nostr_local/registrar/target/registrar.wasm'
STATE = '/tmp/nostr_local/nostr_state.bin'
CONTRACT = 'reg.test.near'

CONTENT = "gm from a NEAR contract (local yield/resume e2e)"
ROOT = "deadbeef" * 8

def sh(cmd, env_extra=None, expect_ok=True):
    env = dict(os.environ)
    env.update(env_extra or {})
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=300, env=env)
    if expect_ok and r.returncode != 0:
        print(f"FAIL rc={r.returncode}\nstdout:\n{r.stdout[-1200:]}\nstderr:\n{r.stderr[-800:]}")
        sys.exit(1)
    return r.stdout + r.stderr

def worker(payload):
    r = subprocess.run([INLAYER, 'run', WORKER, '-i', json.dumps(payload)],
                       capture_output=True, text=True, timeout=300)
    for line in r.stdout.splitlines():
        if 'Output:' in line:
            return line.split('Output: ', 1)[1].strip()
    print("worker produced no output:", r.stderr[-400:]); sys.exit(1)

def cross(method, args_json, signer, view=False, expect_ok=True):
    if os.path.exists(STATE) and method == '__fresh__':
        os.unlink(STATE)
    cmd = [MOCK, 'cross', STATE, f'{CONTRACT}={REG}', CONTRACT, method, args_json]
    if view:
        cmd.append('--view')
    return sh(cmd, {'NEAR_MOCK_SIGNER': signer}, expect_ok=expect_ok)

def dump():
    r = subprocess.run([MOCK, 'state', 'dump', STATE], capture_output=True, text=True)
    return json.loads(r.stdout)

# ── step 0: fresh state ────────────────────────────────────────────────
if os.path.exists(STATE):
    os.unlink(STATE)

# ── step 1: worker derives alice's nostr identity (case N) ─────────────
pk, flip, sk = worker({"case": "N", "root": ROOT, "caller": "alice.test.near"}).split("|")
print(f"[1] derived alice identity pk={pk[:16]}… flip={flip}")

# ── step 2: alice posts (attests predecessor, yields to signer) ────────
out = cross('post', json.dumps({"content": CONTENT, "pk": pk}), 'alice.test.near')
yield_idx = None
for line in out.splitlines():
    if line.startswith('📄'):
        yield_idx = line[1:].strip().strip('"')
print(f"[2] post ok, yield idx = {yield_idx}")
assert yield_idx is not None, "no yield idx returned"

# NotReady first leg of on_sign runs fail-closed: logs NOT_READY, stores nothing
assert 'NOT_READY' in out, "expected NotReady leg marker in post output"
print("    NotReady leg ran fail-closed (NOT_READY, no state written)")

# ── step 3: driver (untrusted) runs the worker: id + sig ───────────────
eid, sig = worker({"case": "E", "pk": pk, "sk": sk, "ts": "0",
                   "kind": "1", "content": CONTENT}).split("|")
print(f"[3] worker signed id={eid[:16]}… sig={sig[:16]}…")

# sanity OFF-chain: independent python verify (what relays will do)
ser = f'[0,"{pk}",0,1,[],"{CONTENT}"]'.encode()
assert hashlib.sha256(ser).hexdigest() == eid, "python id mismatch"
sys.path.insert(0, '/tmp/nostr_probe')
import bip340_reference as ref
assert ref.schnorr_verify(bytes.fromhex(eid), bytes.fromhex(pk), bytes.fromhex(sig)), "python sig verify failed"
print("    off-chain independent verify: id ✓ sig ✓")

# ── step 4: resume the yield with the payload ──────────────────────────
out = cross('resume', json.dumps({"id": yield_idx, "payload": f"{eid}|{sig}"}), CONTRACT)
print(f"[4] resumed:\n    " + "\n    ".join(l for l in out.splitlines() if '📄' in l or 'yield' in l or 'SIGNED' in l))
assert 'ERR_' not in out, f"resume hit an error path: {out[-500:]}"

# ── step 5: assert on STATE (never on markers) ─────────────────────────
rows = dump()
import base64
def val_of(key):
    for r in rows:
        if base64.b64decode(r['key']).decode('utf-8', 'replace') == key:
            return base64.b64decode(r['value']).decode('utf-8', 'replace')
    return None

stored_pk = val_of('pk:alice.test.near')
stored_last = val_of('last:alice.test.near')
print(f"[5] state: pk stored = {stored_pk}")
last = json.loads(stored_last)
assert stored_pk == pk, "pk mismatch in state"
assert last['id'] == eid and last['sig'] == sig, "event mismatch in state"
assert last['content'] == CONTENT, "content mismatch in state"
print(f"    last event: id={last['id'][:16]}… content={last['content']!r}")

# ── step 6: tamper test — FORGED sig must be rejected on-chain ─────────
# The handle from step 2 is consumed, so make a fresh one: copy state,
# post() again (new live yield idx 0), then resume with a forged sig.
if os.path.exists(STATE + '.tamper'):
    os.unlink(STATE + '.tamper')
import shutil; shutil.copy(STATE, STATE + '.tamper')
env = dict(os.environ, NEAR_MOCK_SIGNER='alice.test.near')
r = subprocess.run([MOCK, 'cross', STATE + '.tamper', f'{CONTRACT}={REG}', CONTRACT,
                    'post', json.dumps({"content": "tamper probe", "pk": pk})],
                   capture_output=True, text=True, env=env, timeout=300)
o6post = r.stdout + r.stderr
assert r.returncode == 0, f"tamper probe post failed: {o6post[-300:]}"
# valid id for the FROZEN content ("tamper probe"), then corrupt the sig:
eid2, sig2 = worker({"case": "E", "pk": pk, "sk": sk, "ts": "0",
                     "kind": "1", "content": "tamper probe"}).split("|")
bad = sig2[:62] + 'ffff'
env2 = dict(env, NEAR_MOCK_SIGNER=CONTRACT)
r = subprocess.run([MOCK, 'cross', STATE + '.tamper', f'{CONTRACT}={REG}', CONTRACT,
                    'resume', json.dumps({"id": "0", "payload": f"{eid2}|{bad}"})],
                   capture_output=True, text=True, env=env2, timeout=300)
o6 = r.stdout + r.stderr
assert 'ERR_SIG_INVALID' in o6, f"forged sig was NOT rejected: {o6[-400:]}"
# and the tampered run stored nothing new (reverting partition = state intact)
rt = subprocess.run([MOCK, 'state', 'dump', STATE + '.tamper'], capture_output=True, text=True)
rows_t = json.loads(rt.stdout)
import base64 as b64
last_t = None
for row in rows_t:
    if b64.b64decode(row['key']).decode('utf-8', 'replace') == 'last:alice.test.near':
        last_t = b64.b64decode(row['value']).decode('utf-8', 'replace')
assert last_t is not None and json.loads(last_t)['content'] == CONTENT, "tampered resume mutated state!"
print("[6] tamper test: forged sig rejected on-chain (ERR_SIG_INVALID), state untouched ✓")

# yield handle consumed (one-shot): second resume of idx 0 must no-op
r = subprocess.run([MOCK, 'cross', STATE, f'{CONTRACT}={REG}', CONTRACT,
                    'resume', json.dumps({"id": "0", "payload": f"{eid}|{sig}"})],
                   capture_output=True, text=True, env=env2, timeout=300)
o2 = r.stdout + r.stderr
assert 'ERR_RESUME_FAILED' in o2, f"replay not clearly rejected: {o2[-300:]}"
print("[7] replay test: consumed yield cannot be resumed again ✓")

print("\n✅ END-TO-END OK — contract attested caller, worker signed, chain verified, state persisted")
