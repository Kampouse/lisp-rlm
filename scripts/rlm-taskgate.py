#!/usr/bin/env python3
"""rlm-taskgate — install dreamed tasks (agent-authored homework).

Two lanes:
  td_  (dream-task)    — practice tasks for existing weaknesses. Cap 2, 48h.
  ta_  (dream-advtask) — adversarial frontier tasks (delegation required,
                         tight budget). Cap 2, 24h.

Both prefixes match the nightly t*.lisp glob, so installs auto-join the
rotation. Ledger: data/rlm/dream/taskgate.json
"""
import json, os, re, shutil, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CANDS = {
    "td_": (os.path.join(ROOT, "candidates", "task-proposal.lisp"),  2, 48),
    "ta_": (os.path.join(ROOT, "candidates", "advtask-proposal.lisp"), 2, 24),
}
TASKS = os.path.join(ROOT, "scripts", "rlm-tasks")
GATE = os.path.join(ROOT, "data", "rlm", "dream", "taskgate.json")

def alive(prefix):
    return sorted(
        (os.path.join(TASKS, f) for f in os.listdir(TASKS)
         if f.startswith(prefix) and f.endswith(".lisp")),
        key=os.path.getmtime)

def evict(prefix, cap, ttl_h):
    now = time.time()
    for p in alive(prefix):
        if now - os.path.getmtime(p) > ttl_h * 3600:
            os.remove(p)
            yield ("expired", os.path.basename(p))
    while True:
        a = alive(prefix)
        if len(a) <= cap:
            break
        os.remove(a[0])
        yield ("evicted-cap", os.path.basename(a[0]))

def main():
    os.makedirs(os.path.dirname(GATE), exist_ok=True)
    ledger = json.load(open(GATE)) if os.path.exists(GATE) else {"events": []}
    events = []
    for prefix, (cand, cap, ttl) in CANDS.items():
        if not os.path.exists(cand):
            continue
        src = open(cand).read()
        m = re.search(r'__trace_id\s+"(%s[a-z0-9_]{1,20})"' % prefix, src)
        slug = m.group(1) if m else None
        if not slug or "run-rlm" not in src or "task-verify" not in src:
            print(f"TASKGATE: invalid {prefix} candidate (slug={slug}) — discarded")
            continue
        events += evict(prefix, cap, ttl)
        dst = os.path.join(TASKS, f"{slug}.lisp")
        if os.path.exists(dst) and os.path.getmtime(cand) <= os.path.getmtime(dst):
            print(f"TASKGATE: {slug} already installed")
            continue
        shutil.copyfile(cand, dst)
        events.append(("installed", slug, time.strftime("%F %T")))
        print(f"TASKGATE: installed {slug} — alive {prefix}: "
              + str([os.path.basename(p) for p in alive(prefix)]))
    if events:
        ledger["events"] = (ledger.get("events", []) + events)[-200:]
        json.dump(ledger, open(GATE, "w"), indent=1)

if __name__ == "__main__":
    main()
