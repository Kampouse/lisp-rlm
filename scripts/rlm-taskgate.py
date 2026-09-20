#!/usr/bin/env python3
"""rlm-taskgate — install dreamed tasks (agent-authored homework).

Lifecycle:
  - candidates/task-proposal.lisp  (probe-passed, written by rlm-dream-task.lisp)
  - install as scripts/rlm-tasks/td_<slug>.lisp   (td_ prefix matches the
    nightly t*.lisp glob, so dreamed tasks auto-join the rotation)
  - CAP: at most MAX_ALIVE dreamed tasks; evict oldest by mtime
  - EXPIRY: dreamed tasks die after 48h (practice problems, not curriculum)
  - ledger: data/rlm/dream/taskgate.json
"""
import json, os, re, shutil, time, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))  # lisp-rlm/
CAND = os.path.join(ROOT, "candidates", "task-proposal.lisp")
TASKS = os.path.join(ROOT, "scripts", "rlm-tasks")
GATE = os.path.join(ROOT, "data", "rlm", "dream", "taskgate.json")
MAX_ALIVE, TTL_H = 2, 48

def dreamed():
    return sorted(
        (os.path.join(TASKS, f) for f in os.listdir(TASKS)
         if f.startswith("td_") and f.endswith(".lisp")),
        key=os.path.getmtime)

def evict(all_td):
    now = time.time()
    for p in all_td:
        if now - os.path.getmtime(p) > TTL_H * 3600:
            os.remove(p)
            yield ("expired", os.path.basename(p))
    while True:
        alive = dreamed()
        if len(alive) <= MAX_ALIVE:
            break
        os.remove(alive[0])
        yield ("evicted-cap", os.path.basename(alive[0]))

def main():
    events = []
    if not os.path.exists(CAND):
        print("TASKGATE: no candidate"); return
    src = open(CAND).read()
    m = re.search(r'__trace_id\s+"(td_[a-z0-9_]{1,20})"', src)
    slug = m.group(1) if m else None
    if not slug or '"' not in src or "run-rlm" not in src:
        print(f"TASKGATE: invalid candidate (slug={slug}) — discarded"); return
    # double-check the verifier discriminates: probe markers live in the
    # candidate header; the lisp side already proved ref/poison.
    if "task-verify" not in src:
        print("TASKGATE: no verifier — discarded"); return

    os.makedirs(os.path.dirname(GATE), exist_ok=True)
    ledger = json.load(open(GATE)) if os.path.exists(GATE) else {"events": []}
    events += evict(dreamed())

    dst = os.path.join(TASKS, f"{slug}.lisp")
    if os.path.exists(dst):  # same slug re-dreamed: replace only if new
        if os.path.getmtime(CAND) <= os.path.getmtime(dst):
            print(f"TASKGATE: {slug} already installed"); return
    shutil.copyfile(CAND, dst)
    events.append(("installed", slug, time.strftime("%F %T")))
    ledger["events"] = (ledger.get("events", []) + events)[-200:]
    json.dump(ledger, open(GATE, "w"), indent=1)
    alive = [os.path.basename(p) for p in dreamed()]
    print(f"TASKGATE: installed {slug} — alive dreamed tasks: {alive}")

if __name__ == "__main__":
    main()
