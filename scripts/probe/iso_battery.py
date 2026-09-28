#!/usr/bin/env python3
"""Battery: random + structured (x, y) pairs through iso_probe.wasm vs Python.
fe-mul/fe-muln are MULMONT: result = x*y*R^-1 mod m, R = 2^270.
Fields (fixed 80 chars): A=fe-mul(x,y) B=fe-muln(x,y) C=fmn(x,c-none)
D=fmn(x,x) E=fmn(x,c-rinvn=1)."""
import subprocess, json, secrets, sys

D = '/tmp/nostr_probe'
B = '/Users/asil/.local/bin/inlayer'
W = f'{D}/iso_probe.wasm'

p = 2**256 - 2**32 - 977
n = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
R = 1 << 270
Rinv_p = pow(R, -1, p)
Rinv_n = pow(R, -1, n)

def run_wasm(payload):
    r = subprocess.run([B, 'run', W, 'run', json.dumps(payload, separators=(',', ':'))],
                       capture_output=True, text=True, timeout=900)
    out = None
    for line in r.stdout.split('\n'):
        if 'Output:' in line:
            out = line.split('Output:')[1].strip()
    return out, r

def words2int(s):
    assert len(s) == 80, f"len {len(s)}: {s[:40]}"
    ws = [int(s[i*10:(i+1)*10]) - 2**30 for i in range(8)]
    v = 0
    for w in ws:
        v = (v << 32) | (w & 0xFFFFFFFF)
    return v

fails = 0
tot = 0
worst = []

def check(tag, got, want, mod):
    global fails, tot
    tot += 1
    ok = (got % mod) == (want % mod)
    if not ok:
        fails += 1
        print(f"  {tag}: WRONG  got {hex(got)[:22]} want {hex(want)[:22]}")
    return ok

# targeted: runP's killer value (Nostr event id)
e_nostr = 0x038597d5c8f21c56d7477b070c90be74bd9a5baa8b4d22630d5a344f51ed35dc
vectors = [e_nostr, 0, 1, p - 1, n - 1, 1 << 255]
NTOT = int(sys.argv[1]) if len(sys.argv) > 1 else 20
vectors += [secrets.randbits(256) for _ in range(NTOT)]

for i, xv in enumerate(vectors):
    x = xv % n
    y = secrets.randbits(256) % n
    payload = {'case': 'X', 'sk': f'{x:064x}', 'msg': f'{y:064x}'}
    out, r = run_wasm(payload)
    if out is None or len(out) < 400:
        print(f"[{i}] RUN FAIL: out={out!r} err={r.stderr[-200:] if r.stderr else ''}")
        fails += 1
        continue
    A, Bq, C, Dq, E = (out[k*80:(k+1)*80] for k in range(5))
    tag = f"[{i}] x={hex(x)[:12]}" + (" <NOSTR-E>" if x == e_nostr % n else "")
    check(tag + ' A', words2int(A), x * y * Rinv_p, p)
    check(tag + ' B', words2int(Bq), x * y * Rinv_n, n)
    check(tag + ' C', words2int(C), x, n)                # x*(R%n)*R^-1 = x
    check(tag + ' D', words2int(Dq), x * x * Rinv_n, n)
    check(tag + ' E', words2int(E), x * Rinv_n, n)       # x*1*R^-1

print(f"\n=== {tot} checks, {fails} failures, {len(vectors)} vectors ===")
