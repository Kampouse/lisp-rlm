#!/usr/bin/env python3
"""rlm-intentions — the standing-intention scheduler (blueprint Layer 2 lite).

Revives the BOOTSTRAP.md intent harness without the defpatch machinery:
tasks become INTENTIONS with lifecycles —

  completable  rate < 90%  → "keep caring until mastered";
                              priority = (1 - rate) × stagnation
  perpetual    rate ≥ 90%  → mastered; keep-fresh only (1 slot/cycle)

done-when: last-10-world solve rate ≥ 90% → ledger event, archived to
perpetual. Output: data/rlm/intentions.json (state) +
data/rlm/intentions-draw.txt (ordered task list for the nightly loop —
walls first, one mastered task for freshness). Pure stdlib, no LLM.
"""
import json, glob, os, re, time
from collections import defaultdict

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TRACES = os.path.join(REPO, "data", "rlm", "traces")
TASKS_DIR = os.path.join(REPO, "scripts", "rlm-tasks")
OUT_JSON = os.path.join(REPO, "data", "rlm", "intentions.json")
OUT_DRAW = os.path.join(REPO, "data", "rlm", "intentions-draw.txt")
LEDGER = os.path.join(REPO, "data", "rlm", "dream", "ledger.jsonl")
WINDOW = 10          # last-N worlds define the live solve rate
MASTERY = 0.90       # done-when threshold
MAX_DRAW = 7         # tasks per cycle (budget)


def worlds_by_task():
    by = defaultdict(list)
    for p in glob.glob(os.path.join(TRACES, "*", "*.json")):
        m = re.match(r"(.+)-\d{6}(?:-.+)?\.json", os.path.basename(p))
        if not m:
            continue
        try:
            w = json.loads(open(p).read().replace("\\", "\\\\"))
        except Exception:
            continue
        w["_ts"] = os.path.getmtime(p)
        by[m.group(1)].append(w)
    for ws in by.values():
        ws.sort(key=lambda w: w["_ts"])
    return by


def main():
    by = worlds_by_task()
    state = json.load(open(OUT_JSON)) if os.path.exists(OUT_JSON) \
        else {"archived": []}

    rows, draw = [], []
    for task, ws in by.items():
        last = ws[-WINDOW:]
        rate = sum(1 for w in last if w.get("completed")) / max(len(last), 1)
        # stagnation: worlds since the last solve (all attempts if never)
        stag = len(ws)
        for w in reversed(ws):
            if w.get("completed"):
                stag = len(ws) - ws.index(w) - 1
                break
        stag_boost = 1.0 + min(stag, 30) / 30.0     # 1.0 … 2.0
        priority = round((1.0 - rate) * stag_boost, 3)
        # mastery needs evidence: n ≥ WINDOW, else a 1/1 dreamed task
        # "achieves" instantly and gets archived on a coin flip
        mastered = rate >= MASTERY and len(ws) >= WINDOW
        kind = "perpetual" if mastered else "completable"

        # done-when fired? (was completable, now ≥ 90%)
        if mastered and task not in state["archived"]:
            state["archived"].append(task)
            with open(LEDGER, "a") as f:
                f.write(json.dumps({"ts": time.time(), "event": "intention",
                                    "intent": task, "status": "achieved",
                                    "rate": round(rate, 2)}) + "\n")

        rows.append({"task": task, "kind": kind, "rate": round(rate, 2),
                     "n": len(ws), "stagnation": stag,
                     "priority": priority})

    rows.sort(key=lambda r: -r["priority"])
    for r in rows:
        # only draw tasks whose .lisp still exists (dreamed tasks get swept;
        # stale worlds would otherwise hold draw slots hostage forever)
        if r["kind"] == "completable" and \
                os.path.exists(os.path.join(TASKS_DIR, r["task"] + ".lisp")):
            draw.append(r["task"])
    # cold-start admission (2026-09-30): task files with zero banked worlds
    # can never enter via worlds_by_task — admit them at top priority so new
    # curriculum stages actually get picked up (e.g. lending ladder t_lend_*)
    have = {r["task"] for r in rows}
    for p in sorted(glob.glob(os.path.join(TASKS_DIR, "t*.lisp"))):
        name = os.path.basename(p)[:-5]
        if name not in have:
            draw.insert(0, name)
    # one mastered task for freshness (rotate by minute-of-hour)
    masters = [r["task"] for r in rows if r["kind"] == "perpetual"]
    if masters:
        draw.append(masters[int(time.time() // 60) % len(masters)])
    draw = draw[:MAX_DRAW]

    json.dump({"updated": time.strftime("%Y-%m-%d %H:%M:%S"),
               "archived": state["archived"], "intentions": rows},
              open(OUT_JSON, "w"), indent=1)
    open(OUT_DRAW, "w").write(
        "\n".join(f"scripts/rlm-tasks/{t}.lisp" for t in draw) + "\n")

    print(f"intentions: {sum(1 for r in rows if r['kind']=='completable')} "
          f"open, {len(masters)} mastered → draw: "
          + ", ".join(t.replace("t2_reverse", "t2*").replace("_", "")
                      for t in draw[:4]) + "…")


if __name__ == "__main__":
    main()
