#!/usr/bin/env python3
"""lp_pnl_probe3.py — settle the span question: mint → walk below range → full burnc.
Does burnc return the FULL 3-cell req (incl. the upper cell invisible to composition),
or only the 2-cell live composition? Determines honest V/HODL semantics for op "pnl".
"""
import os, sys, json, subprocess, shutil, base64

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import lp_pnl_probe2 as p2
from lp_pnl_probe2 import build, World, G, dx, dy, reqx, reqy, TWO64

A1, B1 = -1, 1
D = 100 * 10**30

def main():
    wasms = build()
    w = World(wasms)

    w.cross("tokx", "new", {}, "owner")
    w.cross("toky", "new", {}, "owner")
    assert w.pool_op("init", {"toka": "tokx", "tokb": "toky"}, signer="owner") == "init-ok"
    FUND = 250 * 10**27
    for tok in ("tokx", "toky"):
        w.cross(tok, "mint", {"to": "alice", "amt": str(FUND)}, "owner")
        w.cross(tok, "ft_transfer_call", {"receiver_id": "pool", "amount": "1000000000000000000", "msg": "seed"})

    req = w.pool_op("req", {"d": str(D), "a": A1, "b": B1})
    rx_s, ry_s = req.removeprefix("rx:").split(":ry:")
    rx, ry = int(rx_s), int(ry_s)
    print(f"custody req: rx={rx}  ry={ry}   (oracle 3-cell: {reqx(A1,B1,D)}/{reqy(A1,B1,D)})")
    w.cross("tokx", "ft_transfer_call", {"receiver_id": "pool", "amount": rx_s, "msg": f"m:{D}:{A1}:{B1}"})
    w.cross("toky", "ft_transfer_call", {"receiver_id": "pool", "amount": ry_s, "msg": f"m:{D}:{A1}:{B1}"})

    # walk below range (same pin as probe2)
    w.cross("toky", "ft_transfer_call", {"receiver_id": "pool",
             "amount": "102400000000000000000000000000", "msg": "sw"})
    rows = w.rows()
    S = int(rows["pool|6|S"])
    sc = max(min(S, G(B1)), G(A1))
    x_live, y_live = dx(D, sc, G(B1)), dy(D, G(A1), sc)
    P = (S * S) >> 64
    print(f"S={S} (G(-1)={G(-1)} G(1)={G(1)})  below range: x_live={x_live} y_live={y_live}")
    print(f"live V = {y_live + x_live*P}   vs 3-cell custodial mark = {ry + rx*P}")

    pnl = w.pool_op("pnl", {"who": "alice", "a": A1, "b": B1})
    print(f"pnl pre-burn: {pnl}")

    # FULL burnc of the walked position — what does alice actually get back?
    out = w.pool_op("burnc", {"d": str(D), "a": A1, "b": B1})
    print(f"burnc ret: {out}")
    rows2 = w.rows()
    ox = int(rows2.get("pool|6|OWX:alice", "0") or 0)
    oy = int(rows2.get("pool|6|OWY:alice", "0") or 0)
    print()
    print("=== VERDICT ===")
    print(f"X owed to alice : {ox}")
    print(f"   vs 3-cell rx : {rx}      (match={ox==rx})")
    print(f"   vs 2-cell x  : {x_live}  (match={ox==x_live})")
    print(f"Y owed to alice : {oy}")
    print(f"   vs 3-cell ry : {ry}")
    feey = int(rows.get("pool|6|FEEY", "0") or 0)
    print(f"FEEY skim pin   : {feey}  (v>>8 = {102400000000000000000000000000>>8})")
    # what is left in escrow / POS after full burn?
    pos = rows2.get(f"pool|6|POS:alice:{A1}:{B1}", "0")
    lq = {k: v for k, v in rows2.items() if "6|LQ" in k and v not in (None, "0", "")}
    esc = {k: v for k, v in rows2.items() if "ESC" in k and v not in (None, "0", "")}
    print(f"POS after burn  : {pos}")
    print(f"LQ books left   : {lq}")
    print(f"escrow left     : {esc}")

if __name__ == "__main__":
    main()
