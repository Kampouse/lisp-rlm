#!/usr/bin/env python3
"""CLMM v5 limb-stack kernel parity driver (TASK-M2).

Builds the concat wasm (limb_math + kernel + dispatch tail), then runs
staged parity batteries against scripts/clmm_v5_ref.py — the ONLY oracle.

  Stage A — explicit edge batteries (single-op calls):
      isqrt & isqrt_dd: p=0/±1 extremes, perfect squares ±1, 4^k ±1 for
      k=0..64, 2^127 / 2^128−1, large-square cases.
      spow: p = 0, ±1..±5, ±7, ±10, ±100, ±1000, ±10000, ±3471/±3478,
      ±2^k (k=1..19), ±799998..±800000 (the known graveyard).
      dy/dx/dyi/dxo: L ∈ {0,1,10^18}, S/S2 extremes incl. equal, both
      directions, S=1, S=2^128−1.
      sfdy/sfdyi/sfdxo/sfdx: dy/dx ∈ {0,1,2,10,10^9,2^62−1,...}, L set
      incl. 0, S extremes.
      fee: amt/bps edge grid incl. 0, 1, 9999, 10^18, bps 0..10000.
  Stage B — shared-PRNG checksum batches (ONE wasm call each):
      isqrt_batch(seed, 1_000_000)   spow_batch(seed, 200_000)
      Checksum = Σ result mod (10^36−11) + first 8 exact results.
  Stage C — segment/fee checksum batches (50_000 each):
      dy_batch dx_batch dyi_batch dxo_batch sfdy_batch sfdx_batch
      sfdyi_batch sfdxo_batch fee_batch.

Shared PRNG (lisp == python): x ← (x²+1) mod (10^36−11).
Stream rules (must match scripts/clmm_v5_dispatch_tail.lisp exactly):
  isqrt:   n_i = x_i (eval then step); x_0 = seed.
  others:  step first, derive case from new x.
  spow:    p = (x mod 1600001) − 800000
  dy/dx:   S = x1+1; i even: S2 = S + (x2>>100); else S2 = S
  dyi/dxo: S = x1+1; i even: S2 = max(1, S−(x2>>100)); else S2 = S
  sfdy/sfdyi: S = x1+1; dy = x2>>74
  sfdxo/sfdx: S = x1+1; dx = x2>>74
  fee:     amt = x mod 10^18; bps = [0,1,100,400,2000,9999,10000][i%7]
  L = [0, 1, 10^18, 5·10^17][i%4]

Usage: parity.py [--stage a|b|c|all] [--isqrt-n N] [--spow-n N]
                 [--seg-n N] [--keep-state]
Exit 0 iff every battery passes.
"""
import argparse
import json
import os
import re
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
import clmm_v5_ref as ref  # noqa: E402  (the ONLY oracle)

MOCK = os.path.join(ROOT, "target", "release", "near-mock")
COMPILE = os.path.join(ROOT, "target", "release", "near-compile")
CONCAT = "/tmp/v5k.lisp"
WASM = "/tmp/v5k.wasm"
STATE = "/tmp/v5_parity_state.bin"

M = 10**36 - 11
U128M1 = 2**128 - 1
C127 = 2**127
BPS_CYCLE = [0, 1, 100, 400, 2000, 9999, 10000]
L_CYCLE = [0, 1, 10**18, 5 * 10**17]

PASS = 0
FAIL = 0


def prng_step(x):
    return (x * x + 1) % M


def build():
    parts = []
    for rel in ("lib/limb_math.lisp", "lib/clmm_v5_kernel.lisp",
                "scripts/clmm_v5_dispatch_tail.lisp"):
        path = os.path.join(ROOT, rel)
        src = open(path).read()
        # python-balance-check (comment-stripped) BEFORE compiling
        code = "\n".join(l.split(";")[0] for l in src.splitlines())
        d = 0
        for ch in code:
            if ch == "(":
                d += 1
            elif ch == ")":
                d -= 1
                assert d >= 0, f"unbalanced ')' in {rel}"
        assert d == 0, f"unbalanced parens ({d}) in {rel}"
        parts.append(src)
    with open(CONCAT, "w") as f:
        f.write("\n".join(parts))
    r = subprocess.run([COMPILE, CONCAT, WASM], capture_output=True, text=True)
    out = r.stdout + r.stderr
    assert "✅" in out and r.returncode == 0, f"compile failed:\n{out}"
    return True


def call(payload, prepaid=200000):
    if os.path.exists(STATE):
        os.remove(STATE)
    env = dict(os.environ)
    env["NEAR_MOCK_STATE"] = STATE
    cmd = [MOCK, WASM, "_run", payload, "--prepaid", str(prepaid)]
    r = subprocess.run(cmd, capture_output=True, text=True, env=env)
    out = r.stdout + r.stderr
    value, gas = None, None
    for line in out.splitlines():
        if line.startswith("📄") and value is None:
            rest = line[1:].strip()
            m = re.match(r'^"((?:[^"\\]|\\.)*)"', rest)
            value = m.group(1) if m else rest.split()[0]
        if line.startswith("⛽") and gas is None:
            m = re.search(r"([\d.]+)\s*Tgas", line)
            if m:
                gas = float(m.group(1))
    assert value is not None, f"no 📄 for {payload}:\n{out[-600:]}"
    return value, gas


def chk(name, payload, expect, prepaid=200000):
    global PASS, FAIL
    got, _ = call(payload, prepaid)
    if got == str(expect):
        PASS += 1
    else:
        FAIL += 1
        print(f"  ❌ {name} {payload[:80]}: got {got[:50]} exp {str(expect)[:50]}")


def batch_chk(name, payload, mirror_results, prepaid=2000000):
    """mirror_results: list of python-computed result ints, len == n."""
    global PASS, FAIL
    t0 = time.time()
    raw, gas = call(payload, prepaid)
    wall = time.time() - t0
    out = json.loads(raw)
    n = int(out["n"])
    acc = 0
    for i, r in enumerate(mirror_results):
        acc = (acc + r) % M
    first8 = [str(r) for r in mirror_results[:8]]
    ok = (n == len(mirror_results)
          and str(out["sum"]) == str(acc)
          and [str(out.get(f"s{k}")) for k in range(8)] == first8)
    if ok:
        PASS += 1
        print(f"  ✅ {name}: n={n} gas={gas:.1f}Tgas wall={wall:.1f}s")
    else:
        FAIL += 1
        print(f"  ❌ {name}: n={n} sum={out['sum']} exp={acc} "
              f"s0={out.get('s0')} exp_s0={first8[0] if first8 else '-'}")
    return ok


# ─── Stage A: edge batteries ───────────────────────────────────────────

def isqrt_edge_cases():
    cases = set()
    for n in range(0, 18):
        cases.add(n)
    for k in range(0, 65):           # 4^k ± 1  (and 4^k itself)
        cases.add(4**k)
        cases.add(4**k - 1)
        cases.add(4**k + 1)
    for r in (1, 2, 3, 999999937, 5 * 10**17, 10**18 - 1, 10**18,
              2**63 - 1, 2**63, 2**63 + 1, 123456789012345678):
        cases.add(r * r)             # perfect squares
        cases.add(r * r - 1)
        cases.add(r * r + 1)
    cases.add(2**127)
    cases.add(2**127 - 1)
    cases.add(2**128 - 1)
    cases.add(2**128 - 2)
    return sorted(cases)


def stage_a():
    print("── Stage A: edge batteries ──")
    icases = isqrt_edge_cases()
    for n in icases:
        e = ref.isqrt_u128(n)
        chk("isqrt", '{"op":"isqrt","n":"%d"}' % n, e, prepaid=2000000)
        chk("isqrt_dd", '{"op":"isqrt_dd","n":"%d"}' % n, e, prepaid=2000000)
    print(f"  isqrt/isqrt_dd edges: {len(icases)} values × 2 algorithms")

    pcases = [0]
    for base in (1, 2, 3, 4, 5, 7, 10, 100, 1000, 10000, 3471, 3478, 65536,
                 524288, 799998, 799999, 800000):
        pcases.extend([base, -base])
    for k in range(1, 20):
        pcases.extend([2**k, -(2**k)])
    for p in pcases:
        chk("spow", '{"op":"spow","p":"%d"}' % p, ref.S_at_point(p),
            prepaid=2000000)
    print(f"  spow edges: {len(pcases)} values (incl. ±800000 graveyard)")

    # segment edges — both directions + equal + extremes
    seg_edges = []
    for L in (0, 1, 10**18):
        for (S, S2) in ((1, 1), (1, 2), (2, 1), (1, U128M1), (U128M1, U128M1),
                        (U128M1, 1), (2**63, 2**63), (2**63, 2**63 + 1),
                        (2**63 + 1, 2**63), (U128M1 - 1, U128M1)):
            seg_edges.append((L, S, S2))
    x = 987654321098765432109876543210
    for i in range(20):              # PRNG-derived pairs (both directions)
        x = prng_step(x)
        S = x + 1
        x = prng_step(x)
        h = x >> 100
        seg_edges.append((L_CYCLE[i % 4], S, S + h))            # up
        seg_edges.append((L_CYCLE[(i + 1) % 4], S, max(1, S - h)))  # down
    for L, S, S2 in seg_edges:
        if S2 >= S:
            chk("dy", '{"op":"dy","L":"%d","S":"%d","S2":"%d"}' % (L, S, S2),
                ref.dy_out(L, S, S2))
            chk("dx", '{"op":"dx","L":"%d","S":"%d","S2":"%d"}' % (L, S, S2),
                ref.dx_in(L, S, S2))
        if S2 <= S:
            chk("dyi", '{"op":"dyi","L":"%d","S":"%d","S2":"%d"}' % (L, S, S2),
                ref.dy_in(L, S, S2))
            chk("dxo", '{"op":"dxo","L":"%d","S":"%d","S2":"%d"}' % (L, S, S2),
                ref.dx_out(L, S, S2))
    print(f"  segment edges: {len(seg_edges)} (L,S,S2) triples × up to 4 ops")

    dyvals = [0, 1, 2, 3, 10, 10**9, 2**62 - 1, 1234567890123456789, 0]
    sfor_edges = []
    for L in (0, 1, 10**18, 5 * 10**17):
        for S in (1, 2, 3, 2**63, 2**127 - 1, U128M1, 10**30 + 7):
            for d in dyvals[:8]:
                sfor_edges.append((L, S, d))
    x = 555555555555555555555555555555
    for i in range(20):
        x = prng_step(x)
        S = x + 1
        x = prng_step(x)
        d = x >> 74
        sfor_edges.append((L_CYCLE[i % 4], S, d))
    for L, S, d in sfor_edges:
        chk("sfdy", '{"op":"sfdy","L":"%d","S":"%d","dy":"%d"}' % (L, S, d),
            ref.S_for_dy(L, S, d))
        chk("sfdyi", '{"op":"sfdyi","L":"%d","S":"%d","dy":"%d"}' % (L, S, d),
            ref.S_for_dy_in(L, S, d))
        chk("sfdxo", '{"op":"sfdxo","L":"%d","S":"%d","dx":"%d"}' % (L, S, d),
            ref.S_for_dx_out(L, S, d))
        chk("sfdx", '{"op":"sfdx","L":"%d","S":"%d","dx":"%d"}' % (L, S, d),
            ref.S_for_dx(L, S, d))
    print(f"  S_for edges: {len(sfor_edges)} (L,S,dy/dx) × 4 fns")

    fee_edges = []
    for amt in (0, 1, 2, 9999, 10000, 10001, 999999999, 10**18 - 1, 10**18):
        for bps in BPS_CYCLE:
            fee_edges.append((amt, bps))
    x = 777777777777777777777777777777
    for i in range(25):
        x = prng_step(x)
        fee_edges.append((x % 10**18, BPS_CYCLE[i % 7]))
    for amt, bps in fee_edges:
        chk("fee", '{"op":"fee","amt":"%d","bps":"%d"}' % (amt, bps),
            ref.fee_on_input(amt, bps))
    print(f"  fee edges: {len(fee_edges)} (amt,bps) pairs")


# ─── Stage B/C mirrors ─────────────────────────────────────────────────

def mirror_isqrt(seed, n):
    x = seed
    res = []
    for i in range(n):
        res.append(ref.isqrt_u128(x))
        x = prng_step(x)
    return res


def mirror_spow(seed, n):
    x = seed
    res = []
    for i in range(n):
        x = prng_step(x)
        res.append(ref.S_at_point((x % 1600001) - 800000))
    return res


def mirror_seg(seed, n, up, op):
    x = seed
    res = []
    for i in range(n):
        x = prng_step(x)
        S = x + 1
        x = prng_step(x)
        h = x >> 100
        if i % 2 == 0:
            S2 = S + h if up else max(1, S - h)
        else:
            S2 = S
        L = L_CYCLE[i % 4]
        if op == "dy":
            res.append(ref.dy_out(L, S, S2))
        elif op == "dx":
            res.append(ref.dx_in(L, S, S2))
        elif op == "dyi":
            res.append(ref.dy_in(L, S, S2))
        else:
            res.append(ref.dx_out(L, S, S2))
    return res


def mirror_sfor(seed, n, op):
    x = seed
    res = []
    for i in range(n):
        x = prng_step(x)
        S = x + 1
        x = prng_step(x)
        d = x >> 74
        L = L_CYCLE[i % 4]
        if op == 0:
            res.append(ref.S_for_dy(L, S, d))
        elif op == 1:
            res.append(ref.S_for_dy_in(L, S, d))
        elif op == 2:
            res.append(ref.S_for_dx_out(L, S, d))
        else:
            res.append(ref.S_for_dx(L, S, d))
    return res


def mirror_fee(seed, n):
    x = seed
    res = []
    for i in range(n):
        x = prng_step(x)
        res.append(ref.fee_on_input(x % 10**18, BPS_CYCLE[i % 7]))
    return res


SEED1 = 123456789012345678901234567890123456
SEED2 = 987654321098765432109876543210987654321 % M


def stage_b(isqrt_n, spow_n):
    print("── Stage B: shared-PRNG checksum batches ──")
    # small gates first
    batch_chk("isqrt_batch[1k]", '{"op":"isqrt_batch","seed":"%d","n":"1000"}' % SEED1,
              mirror_isqrt(SEED1, 1000))
    batch_chk("spow_batch[1k]", '{"op":"spow_batch","seed":"%d","n":"1000"}' % SEED2,
              mirror_spow(SEED2, 1000))
    print("  gates passed — full runs:")
    batch_chk("isqrt_batch[%d]" % isqrt_n,
              '{"op":"isqrt_batch","seed":"%d","n":"%d"}' % (SEED1, isqrt_n),
              mirror_isqrt(SEED1, isqrt_n), prepaid=3000000)
    batch_chk("spow_batch[%d]" % spow_n,
              '{"op":"spow_batch","seed":"%d","n":"%d"}' % (SEED2, spow_n),
              mirror_spow(SEED2, spow_n), prepaid=3000000)


def stage_c(seg_n):
    print("── Stage C: segment + fee batches ──")
    for op, up in (("dy", 1), ("dx", 1), ("dyi", 0), ("dxo", 0)):
        batch_chk("%s_batch[%d]" % (op, seg_n),
                  '{"op":"%s_batch","seed":"%d","n":"%d"}' % (op, SEED1, seg_n),
                  mirror_seg(SEED1, seg_n, up, op))
    for op, name in ((0, "sfdy"), (1, "sfdyi"), (2, "sfdxo"), (3, "sfdx")):
        batch_chk("%s_batch[%d]" % (name, seg_n),
                  '{"op":"%s_batch","seed":"%d","n":"%d"}' % (name, SEED2, seg_n),
                  mirror_sfor(SEED2, seg_n, op))
    batch_chk("fee_batch[%d]" % seg_n,
              '{"op":"fee_batch","seed":"%d","n":"%d"}' % (SEED1, seg_n),
              mirror_fee(SEED1, seg_n))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--stage", default="all", choices=["a", "b", "c", "all"])
    ap.add_argument("--isqrt-n", type=int, default=1_000_000)
    ap.add_argument("--spow-n", type=int, default=200_000)
    ap.add_argument("--seg-n", type=int, default=50_000)
    args = ap.parse_args()

    t0 = time.time()
    build()
    print(f"build ok → {WASM}")
    if args.stage in ("a", "all"):
        stage_a()
    if args.stage in ("b", "all"):
        stage_b(args.isqrt_n, args.spow_n)
    if args.stage in ("c", "all"):
        stage_c(args.seg_n)
    print(f"══ TOTAL: PASS={PASS} FAIL={FAIL} "
          f"wall={time.time()-t0:.1f}s ══")
    sys.exit(0 if FAIL == 0 else 1)


if __name__ == "__main__":
    main()
