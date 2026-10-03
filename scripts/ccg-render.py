#!/usr/bin/env python3
"""Render the RLM CCG (concept-completion graph) as a PNG.
Chains (nodes touched by supersedes edges) get a force layout and pop;
isolates are placed in faint family-colored rings around the core."""
import json, math, random, collections
import networkx as nx
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.lines import Line2D

random.seed(7)
G_PATH = "data/rlm/ccg/graph.json"
OUT = "/Users/asil/.openclaw/workspace/rlm-ccg.png"

g = json.load(open(G_PATH))
nodes = dict(g["nodes"]); edges = g["edges"]

def family(t):
    return "t_lend" if t.startswith("t_lend") else t.split("_")[0]

fams = sorted({family(v["task"]) for v in nodes.values()})
palette = {
    "ta": "#e15759", "td": "#4e79a7", "t5": "#f28e2b", "t3": "#76b7b2",
    "t2": "#59a14f", "t_lend": "#b07aa1", "t4": "#edc948", "t1": "#9c755f",
    "decsmoke": "#bab0ac", "isoprobe": "#7f7f7f",
}
fam_of = {k: family(v["task"]) for k, v in nodes.items()}
ok_of = {k: bool(v.get("ok")) for k, v in nodes.items()}

G = nx.DiGraph()
G.add_nodes_from(nodes.keys())
G.add_edges_from([(e[0], e[1]) for e in edges])
in_edges = {n for c in nx.weakly_connected_components(G) if len(c) > 1 for n in c}
iso = [n for n in G.nodes if n not in in_edges]

# --- connected core: spring layout ---
core = G.subgraph(in_edges)
pos = nx.spring_layout(core, k=0.35, iterations=60, seed=11)
# rescale core to unit disk
xs = [p[0] for p in pos.values()]; ys = [p[1] for p in pos.values()]
cx, cy = sum(xs)/len(xs), sum(ys)/len(ys)
rmax = max(max(abs(x-cx), abs(y-cy)) for x, y in zip(xs, ys)) or 1
pos = {n: ((x-cx)/rmax*0.72, (y-cy)/rmax*0.72) for n, (x, y) in pos.items()}

# --- isolates: family sectors in outer rings ---
fam_counts = collections.Counter(fam_of[n] for n in iso)
order = sorted(fam_counts, key=lambda f: -fam_counts[f])
fam_angle = {}
total = 2*math.pi
for f in order:
    fam_angle[f] = (total*fam_counts[f]/len(iso), f)
ang = 0.0
for f in order:
    share, _ = fam_angle[f]
    for i, n in enumerate([x for x in iso if fam_of[x] == f]):
        a0 = ang + share*(i/max(1, fam_counts[f]-1))
        r = 0.92 + 0.06*random.random()
        pos[n] = (r*math.cos(a0), r*math.sin(a0))
    ang += share

# --- draw ---
fig, ax = plt.subplots(figsize=(16, 13), facecolor="#0e1116")
ax.set_facecolor("#0e1116")

# edges
for a, b, *_ in edges:
    if a in pos and b in pos:
        x1, y1 = pos[a]; x2, y2 = pos[b]
        ax.plot([x1, x2], [y1, y2], color="#3a4048", lw=0.4, alpha=0.55, zorder=1)

# isolates: faint small dots
for n in iso:
    x, y = pos[n]
    ax.scatter(x, y, s=3, c=palette.get(fam_of[n], "#888"), alpha=0.38,
               linewidths=0, zorder=2)

# chain nodes: size by degree, solved=solid / failed=hollow
deg = dict(core.degree())
for ok in (False, True):
    ns = [n for n in core.nodes if ok_of[n] == ok]
    sizes = [6 + 7*deg[n] for n in ns]
    if ok:
        ax.scatter([pos[n][0] for n in ns], [pos[n][1] for n in ns],
                   s=sizes, c=[palette.get(fam_of[n], "#888") for n in ns],
                   alpha=0.95, linewidths=0, zorder=3)
    else:
        ax.scatter([pos[n][0] for n in ns], [pos[n][1] for n in ns],
                   s=sizes, facecolors="none",
                   edgecolors=[palette.get(fam_of[n], "#888") for n in ns],
                   linewidths=0.6, alpha=0.9, zorder=3)

# label the biggest chain
wccs = sorted(nx.weakly_connected_components(core), key=len, reverse=True)
big = wccs[0]
bt = collections.Counter(fam_of[n] for n in big).most_common(1)[0]
task_names = collections.Counter(nodes[n]["task"] for n in big).most_common(1)[0][0]
bx = sum(pos[n][0] for n in big)/len(big); by = sum(pos[n][1] for n in big)/len(big)
ax.annotate(f"largest chain: {len(big)} nodes\n{task_names}", (bx, by),
            xytext=(bx+0.16, by+0.16), fontsize=10, color="#e8e8e8",
            arrowprops=dict(arrowstyle="-", color="#777", lw=0.8))

n_ok = sum(ok_of.values())
legend = [Line2D([0], [0], marker="o", ls="", color=palette[f],
                 label=f"{f} ({sum(1 for v in nodes.values() if family(v['task'])==f)})",
                 markersize=8) for f in order]
legend += [
    Line2D([0], [0], marker="o", ls="", markerfacecolor="w", color="#e8e8e8",
           label=f"solved attempt (solid) — {n_ok}", markersize=8),
    Line2D([0], [0], marker="o", ls="", markerfacecolor="none", color="#e8e8e8",
           label=f"failed attempt (hollow) — {len(nodes)-n_ok}", markersize=8),
    Line2D([0], [0], color="#3a4048", label="supersedes edge"),
]
leg = ax.legend(handles=legend, loc="upper left", fontsize=9, framealpha=0.25,
                facecolor="#0e1116", labelcolor="#d5d9e0", ncol=2)
ax.set_title("RLM Concept-Completion Graph — 4,691 attempts, 1,227 supersedes",
             color="#e8e8e8", fontsize=15, pad=14)
ax.text(0.99, 0.01, "core: force layout of lineage chains · ring: solved one-shot attempts by task family",
        transform=ax.transAxes, ha="right", fontsize=9, color="#8a919c")
ax.set_xlim(-1.12, 1.12); ax.set_ylim(-1.12, 1.12)
ax.set_aspect("equal"); ax.axis("off")
plt.tight_layout()
plt.savefig(OUT, dpi=170, facecolor="#0e1116", bbox_inches="tight")
print("saved", OUT)
