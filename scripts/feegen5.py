#!/usr/bin/env python3
"""fix bare-number str-cat in mint-finish snapshot writes (trap source)."""
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

old = """              (p5-put (p5-fskey sender a b i) (p5-bz (str-cat "6|FGX:" i)))
              (p5-put (p5-fskeyy sender a b i) (p5-bz (str-cat "6|FGY:" i)))"""
new = """              (p5-put (p5-fskey sender a b i)
                      (p5-bz (str-cat "6|FGX:" (to-string i))))
              (p5-put (p5-fskeyy sender a b i)
                      (p5-bz (str-cat "6|FGY:" (to-string i))))"""
n = src.count(old)
if n != 1:
    fails.append(f"mint-snap: anchor x{n}")
elif net(old) != net(new):
    fails.append(f"mint-snap: paren delta {net(old)-net(new)}")
else:
    src = src.replace(old, new)
    print("ok: mint-snap to-string")

if fails:
    print("FAILED:")
    for f in fails:
        print(" ", f)
    raise SystemExit(1)
open(P, "w").write(src)
print("net:", net(src), "| written")
