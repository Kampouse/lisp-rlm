#!/usr/bin/env python3
"""fix feegen buffer sizes: v5-divmod! needs 10-limb ops in >=12-limb bufs.
q/r -> buf-alloc 64 (16 limbs), zeroed 12/10; dividend+divisor -> 48B (12 limbs).
Anchors from current HEAD+surgery state; net-preserving rewrites."""
import sys

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

def sub(old, new, tag):
    global src
    n = src.count(old)
    if n != 1:
        fails.append(f"{tag}: anchor x{n}")
        return
    if net(old) != net(new):
        fails.append(f"{tag}: paren delta {net(old)-net(new)}")
        return
    src = src.replace(old, new)
    print(f"ok: {tag}")

# ── 1. p5-shr8: 48B dividend/divisor, 64B q/r ──
sub("""(define (p5-shr8 v)
  (let ((A (str->limbs v)) (tA (buf-alloc 32)) (tB (buf-alloc 8))
        (q (buf-alloc 32)) (r (buf-alloc 8)) (one (buf-alloc 8))
        (nbs (buf-alloc 64)) (s (buf-alloc 400)))
    (begin
      (v5-zero! tA 8) (v5-copy! (car A) tA (car (cdr A)))
      (v5-zero! tB 2) (limb-set! tB 0 256)
      (v5-zero! q 8) (v5-zero! r 2)
      (v5-divmod! tA tB q r nbs s one)
      (limbs->str q (if (> (v5-strip q 8) 0) (v5-strip q 8) 1)))))""",
"""(define (p5-shr8 v)
  (let ((A (str->limbs v)) (tA (buf-alloc 48)) (tB (buf-alloc 48))
        (q (buf-alloc 64)) (r (buf-alloc 64)) (one (buf-alloc 8))
        (nbs (buf-alloc 64)) (s (buf-alloc 400)))
    (begin
      (v5-zero! tA 12) (v5-copy! (car A) tA (car (cdr A)))
      (v5-zero! tB 12) (limb-set! tB 0 256)
      (v5-zero! q 12) (v5-zero! r 10)
      (v5-divmod! tA tB q r nbs s one)
      (limbs->str q (if (> (v5-strip q 8) 0) (v5-strip q 8) 1)))))""",
    "shr8")

# ── 2. p5-feegrow: divisor copied into 12-limb tB; q/r 64B ──
sub("""      (let ((FA (str->limbs F)) (LA (str->limbs L))
            (tF (buf-alloc 32)) (tC (buf-alloc 48)) (C64 (buf-alloc 16))
            (q (buf-alloc 32)) (r (buf-alloc 16)) (one (buf-alloc 8))
            (nbs (buf-alloc 64)) (s (buf-alloc 400)))
        (begin
          (v5-zero! tF 8) (v5-copy! (car FA) tF (car (cdr FA)))
          (p5-C64! C64)
          (v5-zero! tC 12)
          (limb-mul tF C64 tC 8 4)
          (v5-zero! q 8) (v5-zero! r 4)
          (v5-divmod! tC (car LA) q r nbs s one)
          (limbs->str q (if (> (v5-strip q 8) 0) (v5-strip q 8) 1))))))""",
"""      (let ((FA (str->limbs F)) (LA (str->limbs L))
            (tF (buf-alloc 48)) (tB (buf-alloc 48)) (tC (buf-alloc 48)) (C64 (buf-alloc 16))
            (q (buf-alloc 64)) (r (buf-alloc 64)) (one (buf-alloc 8))
            (nbs (buf-alloc 64)) (s (buf-alloc 400)))
        (begin
          (v5-zero! tF 12) (v5-copy! (car FA) tF (car (cdr FA)))
          (v5-zero! tB 12) (v5-copy! (car LA) tB (car (cdr LA)))
          (p5-C64! C64)
          (v5-zero! tC 12)
          (limb-mul tF C64 tC 8 4)
          (v5-zero! q 12) (v5-zero! r 10)
          (v5-divmod! tC tB q r nbs s one)
          (limbs->str q (if (> (v5-strip q 8) 0) (v5-strip q 8) 1))))))""",
    "feegrow")

# ── 3. p5-mulshr64: dividend tC 64B; q/r 64B ──
sub("""(define (p5-mulshr64 x y)
  (let ((XA (str->limbs x)) (YA (str->limbs y))
        (tC (buf-alloc 48)) (C64 (buf-alloc 16))
        (q (buf-alloc 32)) (r (buf-alloc 16)) (one (buf-alloc 8))
        (nbs (buf-alloc 64)) (s (buf-alloc 400)))
    (begin
      (v5-zero! tC 12)
      (limb-mul (car XA) (car YA) tC (car (cdr XA)) (car (cdr YA)))
      (p5-C64! C64)
      (v5-zero! q 8) (v5-zero! r 4)
      (v5-divmod! tC C64 q r nbs s one)
      (limbs->str q (if (> (v5-strip q 8) 0) (v5-strip q 8) 1)))))""",
"""(define (p5-mulshr64 x y)
  (let ((XA (str->limbs x)) (YA (str->limbs y))
        (tB (buf-alloc 48)) (tC (buf-alloc 64)) (C64 (buf-alloc 48))
        (q (buf-alloc 64)) (r (buf-alloc 64)) (one (buf-alloc 8))
        (nbs (buf-alloc 64)) (s (buf-alloc 400)))
    (begin
      (v5-zero! tC 16)
      (limb-mul (car XA) (car YA) tC (car (cdr XA)) (car (cdr YA)))
      (v5-zero! tB 12) (v5-copy! (car C64) tB 0)
      (p5-C64! C64)
      (v5-zero! tB 12) (limb-set! tB 0 0)
      (v5-copy! (car C64) tB (car (cdr C64)))
      (v5-zero! q 12) (v5-zero! r 10)
      (v5-divmod! tC tB q r nbs s one)
      (limbs->str q (if (> (v5-strip q 8) 0) (v5-strip q 8) 1)))))""",
    "mulshr64")

# ── 4. p5-quotk: same resizing ──
sub("""(define (p5-quotk F k)
  (let ((FA (str->limbs F)) (tA (buf-alloc 32)) (tB (buf-alloc 8))
        (q (buf-alloc 32)) (r (buf-alloc 8)) (one (buf-alloc 8))
        (nbs (buf-alloc 64)) (s (buf-alloc 400)))
    (begin
      (v5-zero! tA 8) (v5-copy! (car FA) tA (car (cdr FA)))
      (v5-zero! tB 2) (limb-set! tB 0 k)
      (v5-zero! q 8) (v5-zero! r 2)
      (v5-divmod! tA tB q r nbs s one)
      (limbs->str q (if (> (v5-strip q 8) 0) (v5-strip q 8) 1)))))""",
"""(define (p5-quotk F k)
  (let ((FA (str->limbs F)) (tA (buf-alloc 48)) (tB (buf-alloc 48))
        (q (buf-alloc 64)) (r (buf-alloc 64)) (one (buf-alloc 8))
        (nbs (buf-alloc 64)) (s (buf-alloc 400)))
    (begin
      (v5-zero! tA 12) (v5-copy! (car FA) tA (car (cdr FA)))
      (v5-zero! tB 12) (limb-set! tB 0 k)
      (v5-zero! q 12) (v5-zero! r 10)
      (v5-divmod! tA tB q r nbs s one)
      (limbs->str q (if (> (v5-strip q 8) 0) (v5-strip q 8) 1)))))""",
    "quotk")

# whole-file final check
def netfile(t):
    return net(t)

if fails:
    print("FAILED:")
    for f in fails:
        print(" ", f)
    sys.exit(1)

open(P, "w").write(src)
print("net:", netfile(src), "| written")
