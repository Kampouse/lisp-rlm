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

    grammar_search(worlds)

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

    # worst states for the dream-action task (low-value, high-visit)
    try:
        import importlib.util as _il
        _sp = _il.spec_from_file_location("q", os.path.join(HERE, "rlm-tasks", "q-values.json"))
        qv = json.load(open(os.path.join(HERE, "rlm-tasks", "q-values.json")))
        rows = []
        for s, acts in qv.items():
            best = max(acts.values()) if acts else 0.0
            rows.append((best, s))
        rows.sort()
        open(os.path.join(DREAM, "worst-states.txt"), "w").write(
            "\n".join(f"{s} — best Q: {b:.2f}" for b, s in rows[:4]))
    except Exception as e:
        pass  # no q-values yet

    write_scoreboard_and_docs(worlds)
    evolve_stamp(worlds)


# ---- DGM evolution bookkeeping (Dream v4: Darwin Gödel Machine lane) ----
# The flywheel already had three self-modification channels (grammar
# variants, A/B-gated custom actions, auto exemplars) plus this new one:
# tactics.txt — the agent's own self-advice, injected into every prompt.
# This bookkeeper unifies them into visible GENERATIONS: any channel
# delta = one generation stamped to archive.jsonl with a fitness
# snapshot (solve rate, last 10 worlds). Champion tactics are kept;
# two consecutive bad tactics gens auto-restore the champion.
ARCHIVE = os.path.join(DREAM, "archive.jsonl")
EVOLVE_STATE = os.path.join(DREAM, "evolve.json")
TACTICS = os.path.join(HERE, "rlm-tasks", "tactics.txt")
TACTICS_BEST = os.path.join(HERE, "rlm-tasks", "tactics.best.txt")


def write_scoreboard_and_docs(worlds):
    """Plain-text scoreboard + TRUE builtin signatures for the tactics mutator."""
    import re
    by_task = {}
    for w in worlds[-80:]:
        by_task.setdefault(w.get("task_id", "?"), []).append(w)
    lines = []
    for t, ws in sorted(by_task.items()):
        sol = sum(1 for w in ws if w.get("completed"))
        it = sum(w.get("iterations", 0) for w in ws) / len(ws)
        lines.append(f"{t}: {sol}/{len(ws)} solved, {it:.1f} avg iters")
    open(os.path.join(DREAM, "scoreboard.txt"), "w").write("\n".join(lines))
    docs = []
    try:
        for ln in open(os.path.join(REPO, "src", "helpers.rs")):
            m = re.match(r'\s*"([^"]+)"\s*=>\s*"(\([^"]+\)[^"]*)"', ln)
            if m:
                docs.append(m.group(2))
        open(os.path.join(DREAM, "docs.txt"), "w").write("\n".join(docs))
    except Exception:
        pass


def _channel_state():
    cur = {"grammar": ""}
    if os.path.exists(VARIANT_FILE):
        cur["grammar"] = open(VARIANT_FILE).read().strip()
    ab = ""
    try:
        ab = json.load(open(os.path.join(DREAM, "ab.json"))).get("phase", "idle")
    except Exception:
        pass
    cur["actions"] = ab + ("|installed"
                           if os.path.exists(os.path.join(HERE, "rlm-tasks",
                                                          "custom-actions.lisp"))
                           else "|held")
    cur["tactics"] = ""
    if os.path.exists(TACTICS):
        cur["tactics"] = str(int(os.path.getmtime(TACTICS)))  # version = mtime
    return cur


def evolve_stamp(worlds):
    st = json.load(open(EVOLVE_STATE)) if os.path.exists(EVOLVE_STATE) \
        else {"gen": 0, "channels": {}, "champ_fit": 0.0, "bad_tactics": 0}
    cur = _channel_state()
    last = worlds[-10:]
    fit = round(sum(1 for w in last if w.get("completed")) / max(len(last), 1), 3)

    if st["gen"] == 0 and not os.path.exists(ARCHIVE):
        st["channels"] = cur  # baseline snapshot, gen 0
        json.dump(st, open(EVOLVE_STATE, "w"))
        return

    fired = [ch for ch, v in cur.items() if st["channels"].get(ch) != v]
    for ch in fired:
        st["gen"] += 1
        row = {"gen": st["gen"], "ts": time.time(), "channel": ch,
               "fitness": fit, "detail": cur[ch][:60]}
        with open(ARCHIVE, "a") as f:
            f.write(json.dumps(row) + "\n")
        log({"event": "evolve", "gen": st["gen"], "channel": ch, "fitness": fit})

    # tactics selection: keep champions, revert drift
    if "tactics" in fired:
        if fit >= st.get("champ_fit", 0.0):
            st["champ_fit"] = fit
            st["bad_tactics"] = 0
            if os.path.exists(TACTICS):
                shutil.copy(TACTICS, TACTICS_BEST)
        else:
            st["bad_tactics"] += 1
            if st["bad_tactics"] >= 2 and os.path.exists(TACTICS_BEST):
                shutil.copy(TACTICS_BEST, TACTICS)
                log({"event": "evolve", "gen": st["gen"], "channel": "tactics",
                     "fitness": fit, "note": "reverted to champion"})
                st["bad_tactics"] = 0
                cur["tactics"] = str(int(os.path.getmtime(TACTICS)))

    st["channels"] = cur
    json.dump(st, open(EVOLVE_STATE, "w"))



# ---- grammar variant search (Dream v3.1) ----
# The g1→g2→g3 jumps proved prompt text is the highest-leverage policy
# dimension. This lane searches it autonomously: variants are encoded in
# POLICY_ID suffixes (g3=full, g3l=lean, g3s=strings-first); the dream
# layer rotates/exploits based on banked-world solve rates. Deploy =
# write grammar-variant.json + regen policy.lisp via gen-grammar.py.
VARIANT_FILE = os.path.join(HERE, "rlm-tasks", "grammar-variant.json")
GVAR = {"full": "-g3", "lean": "-g3l", "strings-first": "-g3s"}
MIN_WORLDS_EXPLORE = 10   # a variant needs this many worlds before it counts as tested
MIN_CURRENT_MOVE = 15     # don't rotate away until current variant has this many


def grammar_variant_of(policy):
    for name, suf in GVAR.items():
        if str(policy).endswith(suf):
            return name
    return None


def grammar_search(worlds):
    try:
        current = json.load(open(VARIANT_FILE)).get("variant", "full")
    except Exception:
        current = "full"
    stats = {}
    for w in worlds:
        v = grammar_variant_of(w.get("policy", ""))
        if v:
            s = stats.setdefault(v, [0, 0, 0.0])
            s[0] += 1 if w.get("completed") else 0
            s[1] += 1
            s[2] += w.get("iterations", 0)
    if not stats:
        return  # no variant-stamped worlds yet — g3 family not in pool
    cur_n = stats.get(current, [0, 0, 0])[1]
    untested = [v for v in GVAR if v != current and stats.get(v, [0, 0, 0])[1] < MIN_WORLDS_EXPLORE]
    target = None
    reason = ""
    if untested and cur_n >= MIN_CURRENT_MOVE:
        target = min(untested, key=lambda v: stats.get(v, [0, 0, 0])[1])
        reason = f"explore: {target} has {stats.get(target, [0,0,0])[1]} worlds, current {current} has {cur_n}"
    elif not untested:
        best = max(GVAR, key=lambda v: (stats[v][0] / max(stats[v][1], 1),
                                        -stats[v][2] / max(stats[v][1], 1)))
        if best != current:
            b, c = stats[best], stats.get(current, [0, 0, 0])
            if b[0] / max(b[1], 1) > c[0] / max(c[1], 1) + 0.049:
                target = best
                reason = (f"exploit: {best} {b[0]}/{b[1]} vs current {current} "
                          f"{c[0]}/{max(c[1],1)}")
    if target:
        json.dump({"variant": target}, open(VARIANT_FILE, "w"))
        subprocess.run([sys.executable, os.path.join(HERE, "gen-grammar.py")],
                       check=True, capture_output=True)
        log({"event": "grammar", "variant": target, "reason": reason,
             "stats": {v: f"{s[0]}/{s[1]}" for v, s in stats.items()}})
        print(f"dream GRAMMAR DEPLOY: {target} ({reason})")
    else:
        print(f"dream grammar: {current} holds "
              f"({', '.join(f'{v} {s[0]}/{s[1]}' for v, s in sorted(stats.items()))})")

if __name__ == "__main__":
    main()
