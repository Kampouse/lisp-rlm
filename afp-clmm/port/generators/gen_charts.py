#!/usr/bin/env python3
"""Pool-shape SVGs for each contract README (make charts).

Reads constants from gen.py/gen3.py (single source of truth) — same numbers
the contracts and pins are built from, so the pictures cannot drift.
Output: contracts/<leg>/<name>/pool.svg, embedded in each README.
"""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import paths
sys.path.insert(0, paths.GENERATORS)
import gen, gen3

W, H = 640, 240
PL, PR, PT, PB = 70, 20, 34, 40   # plot margins

def fmt(v):
    if v >= 10**23: return f"{v/10**23:g}e23"
    if v >= 10**22: return f"{v/10**22:g}e22"
    return f"{v:g}"

def chart(name, grid, liq, fee, note=""):
    gmin, gmax = grid[0], grid[-1]
    lmax = max(liq)
    x = lambda g: PL + (g - gmin) / (gmax - gmin) * (W - PL - PR)
    yb = H - PB
    h  = lambda L: L / lmax * (yb - PT - 26)
    s  = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" font-family="ui-monospace,Menlo,monospace">']
    s.append(f'<rect width="{W}" height="{H}" fill="#ffffff"/>')
    s.append(f'<text x="{PL}" y="20" font-size="15" font-weight="bold" fill="#111">{name}</text>')
    s.append(f'<text x="{PL+8+len(name)*9}" y="20" font-size="12" fill="#555">fee φ = {fee}</text>')
    if note:
        s.append(f'<text x="{W-PR}" y="20" font-size="11" fill="#888" text-anchor="end">{note}</text>')
    # bars
    for i, L in enumerate(liq):
        x0, x1, bh = x(grid[i]), x(grid[i+1]), h(L)
        fill = "#4c8dff" if i % 2 == 0 else "#7aa7ff"
        s.append(f'<rect x="{x0+1:.1f}" y="{yb-bh:.1f}" width="{x1-x0-2:.1f}" height="{bh:.1f}" fill="{fill}" stroke="#2b6fe3"/>')
        s.append(f'<text x="{(x0+x1)/2:.1f}" y="{yb-bh-6:.1f}" font-size="11" fill="#124" text-anchor="middle">L={fmt(L)}</text>')
    # axes + ticks
    s.append(f'<line x1="{PL-4}" y1="{yb}" x2="{W-PR}" y2="{yb}" stroke="#333"/>')
    s.append(f'<line x1="{PL-4}" y1="{yb}" x2="{PL-4}" y2="{PT}" stroke="#333"/>')
    s.append(f'<text x="6" y="{(yb+PT)//2}" font-size="11" fill="#333" transform="rotate(-90 12 {(yb+PT)//2})">liquidity / interval</text>')
    for g in grid:
        gx = x(g)
        s.append(f'<line x1="{gx:.1f}" y1="{yb}" x2="{gx:.1f}" y2="{yb+4}" stroke="#333"/>')
        s.append(f'<text x="{gx:.1f}" y="{yb+16}" font-size="11" fill="#333" text-anchor="middle">{g/10**9:g}e9</text>')
    s.append(f'<text x="{(PL+W-PR)//2}" y="{H-6}" font-size="11" fill="#333" text-anchor="middle">sqrt-price grid (grd P i)</text>')
    s.append('</svg>')
    return "\n".join(s)

CHARTS = [
    ("pa",   gen.GA, gen.LA, "0.3%"),
    ("pb",   gen.GB, gen.LB, "0.3%", "different grid"),
    ("pd",   gen.GJ, gen.LJ, "0.3%", "join: cell liq = a+b density, B cut at 2e9"),
    ("pc",   gen.GA, gen.LC, "0.5%"),
    ("pj",   gen.GA, [gen.LA[i]+gen.LC[i] for i in range(2)], "mixed", "LK=a+c; Fhat per cell"),
    ("n1",   gen3.G3, gen3.LIQS[1], "0.3%"),
    ("n2",   gen3.G3, gen3.LIQS[2], "0.3%"),
]
for acct, g, l, fee, *note in CHARTS:
    d = paths.cdir(acct)
    with open(os.path.join(d, "pool.svg"), "w") as f:
        f.write(chart(paths.MAP[acct][1], g, l, fee, note[0] if note else ""))
    print("wrote", paths.MAP[acct][0] + "/" + paths.MAP[acct][1] + "/pool.svg")
print("CHARTS DONE")
