#!/usr/bin/env python3
"""Q-table renders: full matrix, argmax policy strip, aggregated playbook, action usage."""
import json, collections
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.patches import Rectangle
from matplotlib.lines import Line2D
import numpy as np

Q = json.load(open("scripts/rlm-tasks/q-values.json"))

def canon(a):
    a = a.replace("~~QT~~", "").strip()
    return a

def parse_state(k):
    parts = k.split("|")
    task = parts[0]
    rest = parts[1:]
    err = rest[0] if rest and rest[0] not in ("",) else "?"
    rc = es = it = ""
    for r in rest:
        if r.startswith("rc"): rc = r
        elif r.startswith("es"): es = r
        elif r.startswith("i"): it = r
    fam = "t_lend" if task.startswith("t_lend") else task.split("_")[0]
    return fam, task, err, rc, es, it

# canonical action columns (merge quoted twins)
actions = collections.Counter()
for v in Q.values():
    for a in v: actions[canon(a)] += 1
cols = [a for a, _ in actions.most_common()]

rows = sorted(Q.keys(), key=lambda k: (parse_state(k)[0], parse_state(k)[2] != "ok",
                                        parse_state(k)[2], parse_state(k)[3],
                                        parse_state(k)[4], parse_state(k)[5]))
M = np.full((len(rows), len(cols)), np.nan)
for i, k in enumerate(rows):
    for a, v in Q[k].items():
        M[i, cols.index(canon(a))] = v

fam_of_row = [parse_state(k)[0] for k in rows]
fam_palette = {"ta":"#e15759","td":"#4e79a7","t5":"#f28e2b","t3":"#76b7b2","t2":"#59a14f",
               "t_lend":"#b07aa1","t4":"#edc948","t1":"#9c755f","decsmoke":"#bab0ac","isoprobe":"#7f7f7f"}
act_palette = plt.cm.tab20(np.linspace(0, 1, 20))
act_color = {a: act_palette[i % 20] for i, a in enumerate(cols)}

fig = plt.figure(figsize=(17, 13), facecolor="#0e1116")
gs = fig.add_gridspec(2, 2, height_ratios=[1, 0.55], hspace=0.28, wspace=0.18)

# ---- Panel A: full matrix ----
axA = fig.add_subplot(gs[0, 0]); axA.set_facecolor("#0e1116")
im = axA.imshow(M, aspect="auto", cmap="RdBu_r", vmin=-1, vmax=1, interpolation="nearest")
axA.set_xticks(range(len(cols))); axA.set_xticklabels(cols, rotation=45, ha="right",
                                                      fontsize=8, color="#d5d9e0")
axA.set_yticks([])
axA.set_title(f"A · full Q-matrix — {len(rows)} states × {len(cols)} actions",
              color="#e8e8e8", fontsize=12)
# family bands on left edge + separators
start = 0
for i in range(1, len(fam_of_row) + 1):
    if i == len(fam_of_row) or fam_of_row[i] != fam_of_row[start]:
        axA.add_patch(Rectangle((-1.6, start), 0.55, i - start,
                                color=fam_palette.get(fam_of_row[start], "#666"),
                                clip_on=False, lw=0))
        if i < len(fam_of_row):
            axA.axhline(i - 0.5, color="#0e1116", lw=1.2)
        start = i
cb = fig.colorbar(im, ax=axA, fraction=0.03, pad=0.02)
cb.ax.tick_params(colors="#d5d9e0", labelsize=7); cb.outline.set_visible(False)

# ---- Panel B: argmax policy strip ----
axB = fig.add_subplot(gs[0, 1]); axB.set_facecolor("#0e1116")
arg = np.nanargmax(M, axis=1)
strip = np.array([[1 if j == arg[i] else np.nan for j in range(len(cols))] for i in range(len(rows))])
axB.imshow(strip, aspect="auto", cmap="gray", vmin=0, vmax=1, interpolation="nearest")
for i in range(len(rows)):
    axB.add_patch(Rectangle((arg[i] - 0.5, i - 0.5), 1, 1,
                 facecolor=act_color[cols[arg[i]]], edgecolor="none"))
    if M[i, arg[i]] <= 0:  # best option is still negative
        axB.plot(arg[i], i, marker="x", color="#e15759", ms=4, mew=1.1)
axB.set_xticks(range(len(cols))); axB.set_xticklabels(cols, rotation=45, ha="right",
                                                      fontsize=8, color="#d5d9e0")
axB.set_yticks([])
axB.set_title("B · argmax policy (same row order) — ✕ = best is still ≤0",
              color="#e8e8e8", fontsize=12)

# ---- Panel C: aggregated playbook (error states only) ----
axC = fig.add_subplot(gs[1, 0]); axC.set_facecolor("#0e1116")
groups = collections.defaultdict(list)
for i, k in enumerate(rows):
    fam, task, err, rc, es, it = parse_state(k)
    if err in ("ok", "?"): continue
    groups[(err, it or "?")].append(i)
gkeys = sorted(groups, key=lambda g: (g[0], g[1]))
P = np.full((len(gkeys), len(cols)), np.nan)
for r, gk in enumerate(gkeys):
    P[r] = np.nanmean(M[groups[gk]], axis=0)
imC = axC.imshow(P, aspect="auto", cmap="RdBu_r", vmin=-1, vmax=1, interpolation="nearest")
axC.set_xticks(range(len(cols))); axC.set_xticklabels(cols, rotation=45, ha="right", fontsize=8, color="#d5d9e0")
axC.set_yticks(range(len(gkeys)))
axC.set_yticklabels([f"{e} @ {i}" for e, i in gkeys], fontsize=8, color="#d5d9e0")
axC.set_title("C · generalized playbook — mean Q by error class × iteration (struggle states only)",
              color="#e8e8e8", fontsize=12)
cb2 = fig.colorbar(imC, ax=axC, fraction=0.03, pad=0.02)
cb2.ax.tick_params(colors="#d5d9e0", labelsize=7); cb2.outline.set_visible(False)

# ---- Panel D: action usage ----
axD = fig.add_subplot(gs[1, 1]); axD.set_facecolor("#0e1116")
usage = collections.Counter(cols[arg[i]] for i in range(len(rows)))
neg = collections.Counter(cols[arg[i]] for i in range(len(rows)) if M[i, arg[i]] <= 0)
order = [a for a, _ in usage.most_common()][::-1]
y = np.arange(len(order))
axD.barh(y, [usage[a] for a in order], color=[act_color[a] for a in order], height=0.72)
axD.barh(y, [neg.get(a, 0) for a in order], color="none", edgecolor="#e15759",
         height=0.72, lw=1.2, label="best ≤ 0 (no good option)")
axD.set_yticks(y); axD.set_yticklabels(order, fontsize=9, color="#d5d9e0")
axD.tick_params(colors="#d5d9e0"); axD.xaxis.label.set_color("#d5d9e0")
for s in axD.spines.values(): s.set_visible(False)
axD.set_title("D · what the policy actually picks (n states)", color="#e8e8e8", fontsize=12)
axD.legend(facecolor="#0e1116", labelcolor="#d5d9e0", fontsize=8, framealpha=0.3)

fig.suptitle("RLM Q-table — learned escalation policy · 315 states", color="#e8e8e8",
             fontsize=15, y=0.995)
plt.savefig("/Users/asil/.openclaw/workspace/rlm-qtable.png", dpi=160,
            facecolor="#0e1116", bbox_inches="tight")
print("saved rlm-qtable.png")

# stats for caption
twins = [a for a in cols if a in ("ladder","stop","none") and f"(~~QT~~{a}~~QT~~)" in Q[list(Q)[0]]]
print("action count:", len(cols))
print("argmax usage:", dict(usage.most_common(6)))
print("states where best<=0:", sum(1 for i in range(len(rows)) if M[i, arg[i]] <= 0), "/", len(rows))
