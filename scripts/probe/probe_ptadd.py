#!/usr/bin/env python3
"""probe_ptadd.py (v3) — aff-x rendering. Expect [2Gx, 2Gx, 3Gx, p-Gx, Gx]."""
import re, subprocess

src = open('/tmp/nostr_probe/gen_bip340_verify.py').read()
pt_add_src = re.search(r"PT_ADD = '''(.*?)'''", src, flags=re.S).group(1)

TEST = open('/tmp/nostr_probe/shamod.lisp').read() + pt_add_src + '''
(define (rep1) (fm (c-onep 0) (c-r2 0)))
(define (serxz P)
  (cat8l (fe-words-pure (fm (aff-x (vec-nth P 0) (vec-nth P 1) (vec-nth P 2)) (c-onep 0)))))
(define (run input)
  (let* ((c (byte-at input 0))
         (z (rep1))
         (g2 (pt-dbl (c-gxm 0) (c-gym 0) z)))
    (if (= c 49)
      (serxz (pt-add (c-gxm 0) (c-gym 0) z (c-gxm 0) (c-gym 0) z))
    (if (= c 50)
      (serxz g2)
    (if (= c 51)
      (serxz (pt-add (c-gxm 0) (c-gym 0) z (vec-nth g2 0) (vec-nth g2 1) (vec-nth g2 2)))
    (if (= c 52)
      (serxz (pt-add (c-gxm 0) (c-gym 0) z (c-gxm 0) (fs (c-zero 0) (c-gym 0)) z))
    (serxz (pt-add (c-gxm 0) (c-gym 0) z (c-zero 0) (c-zero 0) (c-zero 0)))))))))
'''
open('/tmp/nostr_probe/probe_ptadd.lisp', 'w').write(TEST)
r = subprocess.run(['/Users/asil/dev/lisp-rlm/target/release/near-compile',
                    '/tmp/nostr_probe/probe_ptadd.lisp', '--target=outlayer-p2',
                    '-o', '/tmp/nostr_probe/probe_ptadd.wasm'],
                   capture_output=True, text=True)
print("compile rc:", r.returncode, r.stderr[-200:] if r.returncode else "")
if r.returncode == 0:
    outs = []
    for i in range(1, 6):
        rr = subprocess.run(['/Users/asil/.local/bin/inlayer', 'run',
                             '/tmp/nostr_probe/probe_ptadd.wasm', str(i)],
                            capture_output=True, text=True, timeout=120)
        line = [l for l in (rr.stdout + rr.stderr).splitlines() if 'Output:' in l]
        outs.append(line[0].split('Output:')[1].strip() if line else '??')
    p = 2**256 - 2**32 - 977
    Gx = 0x79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798
    Gy = 0x483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8
    def add(P, Q):
        if P is None: return Q
        if Q is None: return P
        if P[0] == Q[0]:
            if (P[1] + Q[1]) % p == 0: return None
            l = (3*P[0]*P[0]) * pow(2*P[1], -1, p) % p
        else:
            l = (Q[1]-P[1]) * pow(Q[0]-P[0], -1, p) % p
        x = (l*l - P[0] - Q[0]) % p
        return (x, (l*(P[0]-x) - P[1]) % p)
    def dbl(P): return add(P, P)
    G = (Gx, Gy); G2 = dbl(G); G3 = add(G, G2)
    want = [G2[0], G2[0], G3[0], p - Gx, Gx]
    for i in range(5):
        w = format(want[i], 'x')
        ok = outs[i].lower().startswith(w[:24].lower()) or ('f2056' in outs[i].lower() if i == 3 else False)
        print(f"case {i+1}: {'OK ' if ok else 'BAD'} got {outs[i][:40]}  want {w[:40]}")
