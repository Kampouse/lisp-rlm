#!/usr/bin/env python3
"""rlm-qlearn — tabular Q-learning over escalation actions (Dream v3).

Mines banked trace worlds for (state, action) decisions recorded per node
by the runtime's Q-layer, applies Q-updates, and regenerates
scripts/rlm-tasks/q-table.lisp for the runtime to consult.

Reward: -0.05 per step, +1.0 terminal bonus if the world completed,
+0.05 shaping for distinct code (exploration bonus).
γ=0.85, α=0.25. Worlds without s/a fields (pre-Q traces) are skipped.

The hand-coded ladder remains the fallback for unseen states and is
itself an action ('ladder') the learner can prefer. No LLM, no human.
"""
import json, os, glob, random

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TRACES = os.path.join(REPO, "data", "rlm", "traces")
QOUT = os.path.join(REPO, "scripts", "rlm-tasks", "q-table.lisp")
LEDGER = os.path.join(REPO, "data", "rlm", "dream", "ledger.jsonl")

ACTIONS = ["none", "temp08", "temp10", "doc", "hint", "ban",
           "budget2", "stop", "ladder", "decompose"]
ALPHA, GAMMA = 0.25, 0.85
STEP_R, SOLVE_R, FAIL_R, NOVEL_R = -0.05, 1.0, -0.2, 0.05
PARTIAL_R = 0.4  # max credit for a near-miss (right idea, wrong details)

# expected answers per task — used for partial-credit scoring of
# incomplete worlds. Answers are compared whitespace/quote/bracket-normalized;
# write-trace banks the literal (to-string answer) in the world's "answer".
EXPECTED = json.load(open(os.path.join(
    REPO, "scripts", "rlm-tasks", "expected.json"))) if os.path.exists(
    os.path.join(REPO, "scripts", "rlm-tasks", "expected.json")) else {}

import re as _re
def _norm(s):
    return _re.sub(r'[\s"\[\],]', '', str(s)).lower()

def partial_score(ans, exp):
    """0..1 closeness of a wrong answer to the expected one.
    1.0 exact · 0.8 same multiset (right content, wrong order/type shape,
    e.g. '(d e s s e r t s)' as a list vs 'desserts' the string) ·
    else sequence similarity."""
    if ans is None or exp is None:
        return 0.0
    a, e = _norm(ans), _norm(exp)
    if not a or not e:
        return 0.0
    if a == e:
        return 1.0
    if sorted(a) == sorted(e):
        return 0.8
    from difflib import SequenceMatcher
    return round(SequenceMatcher(None, a, e).ratio(), 3)


def load_q(path=QOUT.replace("q-table.lisp", "q-values.json")):
    if os.path.exists(path):
        return json.load(open(path))
    return {}


def save_q(q, path=QOUT.replace("q-table.lisp", "q-values.json")):
    json.dump(q, open(path, "w"), indent=0)


def episodes():
    import hashlib
    seen = set()
    paths = glob.glob(os.path.join(TRACES, "*", "*.json")) + \
        glob.glob(os.path.join(TRACES, "*.json"))
    for p in paths:
        raw = open(p, "rb").read()
        h = hashlib.sha256(raw).hexdigest()
        if h in seen:  # dated copy + base file have identical content
            continue
        seen.add(h)
        try:
            w = json.loads(raw.decode().replace("\\", "\\\\"))
        except Exception:
            continue
        nodes = w.get("nodes", [])
        if not nodes or not all("s" in n and "a" in n for n in nodes):
            continue  # pre-Q world
        for n in nodes:  # dream-action installs custom actions at runtime;
            a = n.get("a")  # the action set must follow what actually ran
            if a and a not in ACTIONS:
                ACTIONS.append(a)
        yield w, nodes


RECENT_EPISODES = 80  # ~ today's runtime generation; see note above

def learn(q):
    visits = {}
    eps = sorted(episodes(), key=lambda wn: wn[0].get("ts", 0))
    for w, nodes in eps[-RECENT_EPISODES:]:
        completed = bool(w.get("completed"))
        seen_codes = set()
        T = len(nodes)
        for t, n in enumerate(nodes):
            # backfill task-id prefix onto pre-v2 bare states (new runtime
            # mints task-prefixed states itself; old traces stored bare).
            # guard: never double-prefix already-prefixed states
            tid = w.get('task_id', 'anon')
            s_raw = n["s"]
            s = s_raw if s_raw.startswith(tid + "|") else f"{tid}|{s_raw}"
            a = n["a"]
            if a not in ACTIONS:
                continue
            q.setdefault(s, {x: 0.0 for x in ACTIONS})
            visits[s] = visits.get(s, 0) + 1
            novel = n.get("code", "") not in seen_codes
            seen_codes.add(n.get("code", ""))
            # Terminal reward once (partial credit: near-miss answers earn
            # up to PARTIAL_R — silent-wrong states get gradient not cliff)
            part = partial_score(w.get("answer"),
                                 EXPECTED.get(w.get("task_id")))
            term_r = (SOLVE_R if completed
                      else FAIL_R + PARTIAL_R * part)
            # MONTE-CARLO target: observed future return. TD bootstrapping
            # (r + gamma*max Q(s')) reads zeros for unseen successor states —
            # with n≈1-3 visits per (s,a) the table never moved. MC credits
            # every visited (s,a) with the outcome actually observed, so one
            # episode lights up its whole path. Visits average in over time.
            r = STEP_R + (NOVEL_R if novel else 0)
            target = r + max(0, T - 1 - t) * STEP_R + term_r
            cur = q[s].get(a, 0.0)
            q[s][a] = cur + ALPHA * (target - cur)
    return q, visits


def write_lisp(q, visits):
    lines = [
        ";; GENERATED by scripts/rlm-qlearn.py — do not hand-edit.",
        ";; Learned escalation policy (tabular Q). Unseen states fall",
        ";; back to 'ladder (the hand-coded bootstrap composite).",
        "(define Q-ENABLED true)",
        "(define Q-EPS 10)",
        "(define Q-TABLE (list",
    ]
    for s in sorted(q, key=lambda x: -visits.get(x, 0)):
        acts = sorted(q[s].items(), key=lambda kv: -kv[1])
        row = "  (list \"%s\" (list %s))" % (
            s, " ".join("(list '%s %s)" % (a, round(v, 4)) for a, v in acts))
        lines.append(row)
    lines.append("))")
    open(QOUT, "w").write("\n".join(lines) + "\n")


def main():
    q = load_q()
    # prune unreachable bare-format states (pre-v2, 5 pipe-fields, no
    # task prefix) — the runtime can never look them up again
    q = {s: v for s, v in q.items() if len(s.split("|")) == 6}
    q, visits = learn(q)
    n_states = len(q)
    write_lisp(q, visits)
    save_q(q)
    # worst states (low-value, high-signal) — refreshed EVERY qlearn pass
    # so the dashboard/mutators never see a stale morning table
    rows = sorted(((max(v.values()) if v else 0.0, s) for s, v in q.items()))
    try:
        with open(os.path.join(os.path.dirname(LEDGER), "worst-states.txt"), "w") as f:
            f.write("\n".join(f"{s} — best Q: {b:.2f}" for b, s in rows[:6]))
    except Exception:
        pass
    with open(LEDGER, "a") as f:
        f.write(json.dumps({"event": "qlearn", "ts": __import__("time").time(),
                            "states": n_states,
                            "episodes_seen": sum(1 for _ in episodes())}) + "\n")
    print(f"qlearn: {n_states} states, table written")


if __name__ == "__main__":
    main()
