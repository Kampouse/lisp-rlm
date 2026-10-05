#!/usr/bin/env python3
"""rlm-replay — Dream-RSI layer 1: replay scoring over banked trace worlds.

Reads data/rlm/traces/<date>/*.json (append-only pool) and scores
exploration-policy variants by RE-TRAVERSING recorded iteration nodes:
no LLM calls, no re-execution — the history is the simulator.

Policy variants scored (v1):
  stop_at_k          — abandon task if not Final by iteration k
  stop_after_r_errs  — abandon after r consecutive error iterations

Score = solves / tasks, and iterations saved vs the recorded baseline.
Outcomes are exact for the recorded trees (the paper's replay trick):
a stopping rule can only truncate what actually happened.

Usage: python3 scripts/rlm-replay.py [--pool data/rlm/traces]
"""
import json, glob, os, re, sys, argparse
from collections import defaultdict

POOL_DEFAULT = os.path.join(os.path.dirname(__file__), "..", "data", "rlm", "traces")

# Our lisp writer NEVER emits backslash escapes (sentinels only), so every
# backslash in an old trace is raw data — blanket-double repairs them.
def tolerant_load(path):
    raw = open(path).read()
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return json.loads(raw.replace('\\', '\\\\'))


def load_pool(pool):
    """All trace worlds, newest-last. Dedup by (task_id, ts)."""
    seen, worlds = {}, []
    for path in sorted(glob.glob(os.path.join(pool, "*", "*.json"))):
        try:
            w = tolerant_load(path)
        except (json.JSONDecodeError, OSError):
            continue
        # reverse the lisp-side sentinels
        for node in w.get("nodes", []):
            for field in ("code", "out"):
                node[field] = (node.get(field, "")
                               .replace("~~BS~~", "\\")
                               .replace("~~NL~~", "\n")
                               .replace("~~QT~~", '"')
                               .replace("~~TAB~~", "\t"))
        # dedup by filename — task-<HHMMSS>.json is unique per cycle; the
        # in-world "ts" field is set at runtime-load time and identical
        # across a task's runs, so (task_id, ts) collapses real worlds.
        key = os.path.basename(path)
        if key in seen:
            continue
        seen[key] = True
        w["_pool_path"] = path
        worlds.append(w)
    return worlds


def replay(worlds, stop_at_k=None, stop_after_r_errs=None):
    """Re-traverse each world under a candidate stopping policy.

    A world 'solves' under the policy iff its Final was set at an
    iteration the policy would have reached. Truncation before Final
    = forced abandon. Returns (solves, tasks, iters_used, iters_would).
    """
    solves = iters_used = iters_would = 0
    for w in worlds:
        nodes = w.get("nodes", [])
        completed = w.get("completed", False)
        # Final is set by the LAST node's execution in completed worlds
        # (loop exits right after). Iteration of Final = len(nodes).
        final_iter = len(nodes) if completed else None
        used = len(nodes)
        would = used
        consecutive_errs = 0
        stopped_at = None
        for i, node in enumerate(nodes, start=1):
            if stop_at_k is not None and i > stop_at_k:
                stopped_at = i
                break
            if not node.get("ok", False):
                consecutive_errs += 1
            else:
                consecutive_errs = 0
            if (stop_after_r_errs is not None
                    and consecutive_errs >= stop_after_r_errs):
                stopped_at = i
                break
            would = i
        solved = completed and stopped_at is None and (
            final_iter is not None and would >= final_iter)
        # iterations consumed under policy (stopped early saves the rest)
        consumed = stopped_at if stopped_at is not None else used
        solves += 1 if solved else 0
        iters_used += used
        iters_would += consumed
    return solves, len(worlds), iters_used, iters_would


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pool", default=POOL_DEFAULT)
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    worlds = load_pool(args.pool)
    if not worlds:
        print("replay: empty pool — nothing to dream on yet")
        return
    by_task = defaultdict(list)
    for w in worlds:
        by_task[w.get("task_id", "?")].append(w)

    base_solves, n, used, _ = replay(worlds)
    print(f"replay pool: {n} worlds across {len(by_task)} tasks "
          f"(baseline solves {base_solves}/{n})")

    rows = []
    for k in (2, 4, 8, 12, 16):
        s, _, _, would = replay(worlds, stop_at_k=k)
        rows.append((f"stop_at_{k}", s, would))
    for r in (2, 3, 5):
        s, _, _, would = replay(worlds, stop_after_r_errs=r)
        rows.append((f"stop_after_{r}_errs", s, would))

    print(f"{'policy':<24} {'solves':>7} {'delta':>6} {'iters':>7} {'saved':>7}")
    for name, s, would in rows:
        print(f"{name:<24} {s:>3}/{n:<3} {s - base_solves:>+6} "
              f"{would:>7} {used - would:>+7}")

    if args.verbose:
        for tid, ws in sorted(by_task.items()):
            for w in ws:
                errs = sum(1 for nd in w["nodes"] if not nd["ok"])
                print(f"  {tid:<16} {w.get('policy','?'):<20} "
                      f"completed={str(w.get('completed')):<5} "
                      f"iters={w.get('iterations')} errs={errs} "
                      f"answer={str(w.get('result'))[:30]}")


if __name__ == "__main__":
    main()
