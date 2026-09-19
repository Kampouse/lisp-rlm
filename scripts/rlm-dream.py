#!/usr/bin/env python3
"""rlm-dream — the autonomous dream layer (Dream-RSI miniature).

Runs at the end of every flywheel cycle. Two lanes:

REPLAY LANE (fully autonomous, deterministic, zero LLM):
  sweep stopping rules (max_iter, stop_after_errs) over the frozen trace
  pool; if the best rule beats the incumbent policy (same solves, fewer
  iterations — or more solves), deploy it: update policy-params.json,
  regenerate policy.lisp via gen-grammar.py, bump POLICY_ID. A ledger
  records every deploy; a revert guard rolls back if the post-deploy
  solve rate collapses.

A/B LANE (LLM dreamer, banked for rotation):
  a lisp dream task reads the pool digest + current grammar and proposes
  a grammar edit; proposals are stamped into candidates/ with pool
  context. Rotation/deployment of text edits = v2 (needs online cycles
  to measure, can't be replayed).

No humans, no agents in the deploy loop. The recorded past decides.
"""
import json, glob, os, sys, subprocess, shutil, time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.join(HERE, "..")
POOL = os.path.join(REPO, "data", "rlm", "traces")
DREAM = os.path.join(REPO, "data", "rlm", "dream")
PARAMS = os.path.join(HERE, "rlm-tasks", "policy-params.json")
LEDGER = os.path.join(DREAM, "ledger.jsonl")
os.makedirs(DREAM, exist_ok=True)

import importlib.util as _ilu
_spec = _ilu.spec_from_file_location("rlm_replay", os.path.join(HERE, "rlm-replay.py"))
_r = _ilu.module_from_spec(_spec)
_spec.loader.exec_module(_r)
load_pool, replay = _r.load_pool, _r.replay

SWEEP_K = (4, 6, 8, 10, 12, 16, 20)
SWEEP_R = (0, 2, 3, 4)


def log(event):
    with open(LEDGER, "a") as f:
        f.write(json.dumps({"ts": time.time(), **event}) + "\n")


def incumbent_params():
    return json.load(open(PARAMS))


def score(worlds, k, r):
    solves, n, used, would = replay(
        worlds, stop_at_k=(k if k else None),
        stop_after_r_errs=(r if r else None))
    return solves, n, would


def deploy(k, r, reason):
    shutil.copy(PARAMS, PARAMS + ".bak")
    params = incumbent_params()
    params.update({"max_iter": int(k), "stop_errs": int(r),
                   "version": params.get("version", 1) + 1})
    json.dump(params, open(PARAMS, "w"), indent=1)
    subprocess.run([sys.executable, os.path.join(HERE, "gen-grammar.py")],
                   check=True, capture_output=True)
    log({"event": "deploy", "max_iter": k, "stop_errs": r,
         "reason": reason, "prev": params.get("version", 1)})


def revert_guard(worlds):
    """If the last deploy preceded a solve-rate collapse, roll back."""
    deploys = [json.loads(l) for l in open(LEDGER)] if os.path.exists(LEDGER) else []
    deploys = [d for d in deploys if d["event"] == "deploy"]
    if not deploys:
        return
    last = deploys[-1]
    new_worlds = [w for w in worlds if w.get("ts", 0) > last["ts"]]
    if len(new_worlds) < 5:
        return  # not enough post-deploy evidence yet
    before = [w for w in worlds if w.get("ts", 0) <= last["ts"]]
    rate_before = sum(1 for w in before if w.get("completed")) / max(len(before), 1)
    rate_after = sum(1 for w in new_worlds if w.get("completed")) / len(new_worlds)
    if rate_after < rate_before - 0.15:
        shutil.copy(PARAMS + ".bak", PARAMS)
        subprocess.run([sys.executable, os.path.join(HERE, "gen-grammar.py")],
                       check=True, capture_output=True)
        log({"event": "revert", "rate_before": round(rate_before, 3),
             "rate_after": round(rate_after, 3)})


def main():
    worlds = load_pool(POOL)
    if len(worlds) < 5:
        print(f"dream: pool too small ({len(worlds)} worlds) — accumulating")
        return

    revert_guard(worlds)

    inc = incumbent_params()
    base_solves, n, base_iters = score(worlds, inc["max_iter"], inc["stop_errs"])
    best = (base_solves, -base_iters, inc["max_iter"], inc["stop_errs"])
    for k in SWEEP_K:
        for r in SWEEP_R:
            if (k, r) == (inc["max_iter"], inc["stop_errs"]):
                continue
            s, _, iters = score(worlds, k, r)
            cand = (s, -iters, k, r)
            if cand > best:
                best = cand
    solves, neg_iters, k, r = best
    if (k, r) != (inc["max_iter"], inc["stop_errs"]):
        deploy(k, r,
               f"solves {solves}/{n} at {neg_iters} iters vs "
               f"incumbent {base_solves}/{n} at {base_iters}")
        print(f"dream DEPLOY: max_iter={k} stop_errs={r} "
              f"({solves}/{n} solves, {-neg_iters} iters)")
    else:
        print(f"dream: incumbent holds ({base_solves}/{n} solves, "
              f"{base_iters} iters) — no deploy")

    # digest for the dream task / humans
    by_policy = {}
    for w in worlds:
        p = w.get("policy", "?")
        by_policy.setdefault(p, []).append(w)
    digest = {
        "worlds": len(worlds),
        "policies": {p: {"n": len(ws),
                         "completed": sum(1 for w in ws if w.get("completed")),
                         "avg_iters": round(sum(w.get("iterations", 0) for w in ws)
                                            / max(len(ws), 1), 2)}
                     for p, ws in by_policy.items()},
        "incumbent": {"max_iter": k, "stop_errs": r},
    }
    json.dump(digest, open(os.path.join(DREAM, "digest.json"), "w"), indent=1)


if __name__ == "__main__":
    main()
