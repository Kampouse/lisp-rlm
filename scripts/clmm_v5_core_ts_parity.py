#!/usr/bin/env python3
"""CLMM v5 core-math differential: lisp p5-dy/dx family vs compilable TS.

Random L and grid pairs (i1 < i2 across ±800000, edge pockets at both
extremes). S values come from the lisp kernel's own G(i) — the pow chain
is lisp-side (needs 512-bit squaring; see fixtures/clmm_core_ts.ts for
the port boundary). Byte-exact gate: zero diffs across dy/dyi/dx/dxo.

Usage: python3 scripts/clmm_v5_core_ts_parity.py [N]   (default N=100)
Builds both wasm fresh each run into a temp dir.
"""
import os
import random
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MOCK = os.path.join(ROOT, "target", "release", "near-mock")
NC = os.path.join(ROOT, "target", "release", "near-compile")

N = int(sys.argv[1]) if len(sys.argv) > 1 else 100

WRAPPERS = r''';; core-math differential doors
(define (hG) (p5-G (str->num (json-get-str "i" (near/input)))))
(define (hdy) (p5-dy (json-get-str "L" (near/input)) (json-get-str "S" (near/input)) (json-get-str "S2" (near/input))))
(define (hdyi) (p5-dyi (json-get-str "L" (near/input)) (json-get-str "S" (near/input)) (json-get-str "S2" (near/input))))
(define (hdx) (p5-dx (json-get-str "L" (near/input)) (json-get-str "S" (near/input)) (json-get-str "S2" (near/input))))
(define (hdxo) (p5-dxo (json-get-str "L" (near/input)) (json-get-str "S" (near/input)) (json-get-str "S2" (near/input))))
(define (hts)
  (let ((dyb (p5-str6 (json-get-str "dy" (near/input))))
        (Lb (p5-str6 (json-get-str "L" (near/input))))
        (tA (buf-alloc 80)) (tB (buf-alloc 80)) (tC (buf-alloc 88))
        (one (buf-alloc 8))
        (q (buf-alloc 64)) (r (buf-alloc 64)) (nbs (buf-alloc 64))
        (s (buf-alloc 400)))
    (begin
      (p5-tstar! dyb Lb tA tB tC q r nbs s one)
      (p5-buf-str q))))
(export "hG" hG)
(export "hdy" hdy)
(export "hdyi" hdyi)
(export "hdx" hdx)
(export "hdxo" hdxo)
(export "hts" hts)
'''


def build(tmp):
    srcs = ["lib/limb_math.lisp", "lib/clmm_v5_kernel.lisp",
            "lib/clmm_pool_v5.lisp"]
    src = ""
    for f in srcs:
        with open(os.path.join(ROOT, f)) as fh:
            src += fh.read() + "\n"
    lisp_src = os.path.join(tmp, "probe.lisp")
    with open(lisp_src, "w") as fh:
        fh.write(src + WRAPPERS)
    outs = {}
    for name, cmd in [
        ("lisp", [NC, lisp_src, os.path.join(tmp, "lisp.wasm")]),
        ("ts", [NC, os.path.join(ROOT, "fixtures/clmm_core_ts.ts"),
                os.path.join(tmp, "ts.wasm")]),
    ]:
        r = subprocess.run(cmd, capture_output=True, text=True)
        if r.returncode != 0:
            sys.exit(f"compile {name} failed: " + r.stderr[-400:])
        outs[name] = cmd[-1]
    return outs


def run(wasm, fn, args):
    r = subprocess.run([MOCK, wasm, fn, args], capture_output=True, text=True)
    for line in r.stdout.splitlines():
        if line.startswith("📄 "):
            v = line[2:].strip()
            # the mock annotates short strings: '"123" (8-byte str | i64 …)'
            if v.startswith('"') and " (" in v:
                v = v[1:v.index('"', 1)]
            return v
    return "TRAP:" + (r.stdout + r.stderr)[-200:]


def main():
    tmp = tempfile.mkdtemp(prefix="clmm_core_parity_")
    w = build(tmp)
    random.seed(20261019)
    fails = cases = 0
    wide_skips = 0
    for t in range(N):
        i1 = random.choice([random.randint(-800000, 800000),
                            random.randint(-100, 100),
                            random.randint(-800000, -799000),
                            random.randint(799000, 800000)])
        i2 = i1 + random.randint(1, max(1, abs(random.randint(-8000, 8000))))
        if i2 > 800000:
            i2 = 800000
        L = str(random.choice([random.randint(1, 10**24),
                               random.randint(10**24, 10**30),
                               10**26]))
        S1 = run(w["lisp"], "hG", '{"i":"%d"}' % i1)
        S2 = run(w["lisp"], "hG", '{"i":"%d"}' % i2)
        if S1.startswith("TRAP") or S2.startswith("TRAP") or S1 == S2:
            continue
        args = '{"L":"%s","S":"%s","S2":"%s"}' % (L, S1, S2)
        # down-move twins expect S > S2 (args swapped per the lisp contract)
        args_dn = '{"L":"%s","S":"%s","S2":"%s"}' % (L, S2, S1)
        # TS surface domain: u256Mul is u128×u128→u256 and u256Shl can't
        # exceed 2^256. The kernel's v5-dx! runs a 15-limb (480-bit)
        # pipeline and is exact everywhere — this gate proves the TS port
        # only inside the surface's width. Skip (counted) where
        # L·(S2−S)·2^64 ≥ 2^256, mirroring the t* domain skip below.
        if int(L) * (int(S2) - int(S1)) * (1 << 64) >= (1 << 256):
            wide_skips += 1
            continue
        for lf, tf, arg in [("hdy", "tsdy", args), ("hdyi", "tsdyi", args_dn),
                            ("hdx", "tsdx", args), ("hdxo", "tsdxo", args_dn)]:
            a = run(w["lisp"], lf, arg)
            b = run(w["ts"], tf, arg)
            cases += 1
            if a != b:
                fails += 1
                print(f"DIFF {lf} L={L} i1={i1} i2={i2}\n  lisp: {a}\n  ts:   {b}")
        # t*: dy from the up-move step, then lisp p5-tstar! vs TS tstar.
        # p5-tstar! holds dy in 4 base-1e9 limbs (max 1e36) — larger dy is
        # outside the kernel's domain and truncates there; skip those.
        dy = run(w["ts"], "tsdy", args)
        if not dy.startswith("TRAP") and len(dy) < 36:
            argt = '{"dy":"%s","L":"%s"}' % (dy, L)
            a = run(w["lisp"], "hts", argt)
            b = run(w["ts"], "tstar", argt)
            cases += 1
            if a != b:
                fails += 1
                print(f"DIFF tstar L={L} i1={i1} i2={i2} dy={dy}\n  lisp: {a}\n  ts:   {b}")
        if (t + 1) % 25 == 0:
            print(f"  {t+1}/{N} cases, {fails} diffs")
    print(f"cases={cases} fails={fails} wide_skips={wide_skips}")
    sys.exit(1 if fails else 0)


if __name__ == "__main__":
    main()
