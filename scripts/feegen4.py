#!/usr/bin/env python3
"""fix p5-mulshr64: remove (car C64)-on-buffer junk; clean (v5-copy! C64 tB 4)."""
P = "lib/clmm_pool_v5.lisp"
src = open(P).read()
fails = []

def net(t):
    total = 0
    for line in t.split("\n"):
        d = 0; s = False; i = 0
        while i < len(line):
            c = line[i]
            if s:
                if c == "\\":
                    i += 2; continue
                if c == '"':
                    s = False
            elif c == '"':
                s = True
            elif c == ";":
                break
            elif c == "(":
                d += 1
            elif c == ")":
                d -= 1
            i += 1
        total += d
    return total

old = """      (v5-zero! tC 16)
      (limb-mul (car XA) (car YA) tC (car (cdr XA)) (car (cdr YA)))
      (v5-zero! tB 12) (v5-copy! (car C64) tB 0)
      (p5-C64! C64)
      (v5-zero! tB 12) (limb-set! tB 0 0)
      (v5-copy! (car C64) tB (car (cdr C64)))
      (v5-zero! q 12) (v5-zero! r 10)"""
new = """      (v5-zero! tC 16)
      (limb-mul (car XA) (car YA) tC (car (cdr XA)) (car (cdr YA)))
      (p5-C64! C64)
      (v5-zero! tB 12) (v5-copy! C64 tB 4)
      (v5-zero! q 12) (v5-zero! r 10)"""
n = src.count(old)
if n != 1:
    fails.append(f"mulshr64-junk: anchor x{n}")
elif net(old) != net(new):
    fails.append(f"mulshr64-junk: paren delta {net(old)-net(new)}")
else:
    src = src.replace(old, new)
    print("ok: mulshr64-junk")

if fails:
    print("FAILED:")
    for f in fails:
        print(" ", f)
    raise SystemExit(1)
open(P, "w").write(src)
print("net:", net(src), "| written")
