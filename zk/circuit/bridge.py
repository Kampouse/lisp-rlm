#!/usr/bin/env python3
"""snarkjs → NEAR alt_bn128 wire-format bridge.

Everything (points, scalars) is a field element encoded as 32 bytes
LITTLE-ENDIAN (nearcore's LE-halves quirk == plain LE; see bn254.rs
decode_u256). G1 = x‖y (64B). G2 = x.c0‖x.c1‖y.c0‖y.c1 (128B).
A is negated here: (x, p−y) — the contract expects negA.
"""
import json
import sys

P = 21888242871839275222246405745257275088696311157297823662689037894645226208583  # base field


def elem(v) -> str:
    return int(v).to_bytes(32, "little").hex()


def g1(p):
    return elem(p[0]) + elem(p[1])


def g1_neg(p):
    x, y = int(p[0]), int(p[1])
    return elem(x) + elem(P - y)  # (x, −y)


def g2(p):
    return elem(p[0][0]) + elem(p[0][1]) + elem(p[1][0]) + elem(p[1][1])


def main(dir="."):
    d = dir.rstrip("/") + "/"
    proof = json.load(open(d + "proof.json"))
    vkey = json.load(open(d + "vkey.json"))
    pub = json.load(open(d + "public.json"))

    n = vkey["nPublic"]
    assert len(pub) == n, f"public signals {len(pub)} != nPublic {n}"

    init = {
        "alpha1": g1(vkey["vk_alpha_1"]),
        "beta2": g2(vkey["vk_beta_2"]),
        "gamma2": g2(vkey["vk_gamma_2"]),
        "delta2": g2(vkey["vk_delta_2"]),
        "n": n,
        "ic": [g1(ic) for ic in vkey["IC"]],
    }
    verify = {
        "negA": g1_neg(proof["pi_a"]),
        "B": g2(proof["pi_b"]),
        "C": g1(proof["pi_c"]),
        "inputs": [elem(s) for s in pub],
    }
    json.dump(init, open(d + "init_args.json", "w"))
    json.dump(verify, open(d + "verify_args.json", "w"))
    print(f"bridge ok: nPublic={n}, init {len(json.dumps(init))}B, verify {len(json.dumps(verify))}B")
    print(f"  Cx={pub[0][:20]}… Cy={pub[1][:20]}…")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else ".")
