#!/usr/bin/env python3
"""rlm-exemplars — auto-mine few-shot exemplars from mastered tasks.

A task is 'mastered' when it solved >=90% of its last 10 worlds. For a
mastered task, the LAST ok node containing 'answer' from the LATEST
completed world becomes its exemplar (own-corpus self-distillation).

Hand-written bootstrap exemplars (files starting with '# hand') are
NEVER overwritten. Files without a '# hand' or '# auto' marker are
treated as hand-authored (untouched) — unknown provenance, be safe.

Called from rlm-nightly.sh after rlm-qlearn.py. Exits 0 always.
"""
import json, os, glob, re, hashlib

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TRACES = os.path.join(REPO, "data", "rlm", "traces")
EXDIR = os.path.join(REPO, "scripts", "rlm-tasks", "exemplars")

SENTINELS = [("~~NL~~", "\n"), ("~~QT~~", '"'), ("~~BS~~", "\\")]


def decode(s):
    for k, v in SENTINELS:
        s = s.replace(k, v)
    return s


def load(p):
    raw = open(p, "rb").read()
    try:
        return json.loads(raw.decode().replace("\\", "\\\\"))
    except Exception:
        return None


def worlds():
    seen, out = set(), []
    for p in glob.glob(os.path.join(TRACES, "*", "*.json")) + \
             glob.glob(os.path.join(TRACES, "*.json")):
        h = hashlib.sha256(open(p, "rb").read()).hexdigest()
        if h in seen:
            continue
        seen.add(h)
        w = load(p)
        if w:
            out.append(w)
    return out


def main():
    os.makedirs(EXDIR, exist_ok=True)
    by_task = {}
    for w in worlds():
        by_task.setdefault(w.get("task_id", "?"), []).append(w)

    written, skipped_hand, skipped_weak = [], [], []
    for task, ws in by_task.items():
        ws.sort(key=lambda w: w.get("ts", 0))
        last10 = ws[-10:]
        if len(last10) < 5:
            continue
        solved = sum(1 for w in last10 if w.get("completed"))
        if solved / len(last10) < 0.9:
            skipped_weak.append(task)
            continue
        # latest completed world → last ok node mentioning answer
        chosen = None
        for w in reversed(ws):
            if not w.get("completed"):
                continue
            for n in reversed(w.get("nodes", [])):
                if n.get("ok") and "answer" in (n.get("code") or "").lower():
                    chosen = decode(n.get("code"))
                    break
            if chosen:
                break
        if not chosen:
            continue
        path = os.path.join(EXDIR, task + ".txt")
        if os.path.exists(path):
            head = open(path).read(64)
            if head.startswith("# hand") or not head.startswith("# auto"):
                skipped_hand.append(task)
                continue
        with open(path, "w") as f:
            f.write("# auto — mined from own completed worlds\n" + chosen + "\n")
        written.append(task)

    print(f"exemplars: {len(written)} written/refreshed, "
          f"{len(skipped_hand)} hand-protected, "
          f"{len(skipped_weak)} not mastered (no exemplar)")
    if written:
        print("  refreshed:", ", ".join(sorted(written)))


if __name__ == "__main__":
    main()
