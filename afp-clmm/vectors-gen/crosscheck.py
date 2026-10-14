#!/usr/bin/env python3
"""Cross-check the machine-checked fixtures (run_vectors output, from
the Isabelle code-generator export of accruef/prodsum) against an
INDEPENDENT Python mirror of the fold. Exit 1 on any mismatch.

The Python mirror is written from the ISAR doc comment in
port/contracts/accrual/accrual/src/main.lisp, not from the SML, so any
export/generator bug shows up as a diff here.
"""
import json
import sys

U64 = 1 << 64


def accruef(lf, c, gs):
    # accruef Lf c [] = 0 ; accruef Lf c (g#gs) = (g-c)*Lf g div 2^64 + ...
    if not gs:
        return 0
    g = gs[0]
    delta = g - c if g >= c else 0  # nat subtraction saturates
    return (delta * lf(g) // U64) + accruef(lf, g, gs[1:])


def prodsum(lf, c, gs):
    if not gs:
        return 0
    g = gs[0]
    delta = g - c if g >= c else 0
    return delta * lf(g) + prodsum(lf, g, gs[1:])


def main():
    fixtures = json.load(sys.stdin)
    bad = 0
    for row in fixtures:
        g = row["g"]
        lf_one = lambda _g: 1
        lf_g = lambda x: x
        checks = {
            "owed_one": accruef(lf_one, 0, [g]),
            "owed_g": accruef(lf_g, 0, [g]),
            "undivided": prodsum(lf_g, 0, [g]),
        }
        for k, want in checks.items():
            if row[k] != want:
                print(f"MISMATCH g={g} {k}: ml={row[k]} py={want}")
                bad += 1
    if bad:
        print(f"FAIL: {bad} mismatches")
        sys.exit(1)
    n = len(fixtures)
    print(f"OK: {n}/{n} fixture rows match the independent mirror")


if __name__ == "__main__":
    main()
