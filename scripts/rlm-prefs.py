#!/usr/bin/env python3
"""rlm-prefs — mine DPO preference pairs from banked RLM trace worlds.

Within-world pairs: a completed world contributes (chosen = last ok node
that stores 'answer', rejected = each distinct earlier node code).
Cross-world pairs: the task's FIRST completed world's chosen vs every
distinct node code from strictly earlier worlds of the same task.

Pairs are written to data/rlm/prefs/pairs.jsonl for future DPO-style
fine-tuning. Sentinels (~~NL~~ / ~~QT~~ / ~~BS~~) are decoded — the
model trains on raw lisp source. Pure stdlib, no LLM, no network.
"""
import json, os, glob, hashlib

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TRACES = os.path.join(REPO, "data", "rlm", "traces")
OUT = os.path.join(REPO, "data", "rlm", "prefs", "pairs.jsonl")

SENTINELS = [("~~NL~~", "\n"), ("~~QT~~", '"'), ("~~BS~~", "\\")]


def decode(s):
    for k, v in SENTINELS:
        s = s.replace(k, v)
    return s


def worlds():
    seen, out = set(), []
    for p in glob.glob(os.path.join(TRACES, "*", "*.json")) + \
             glob.glob(os.path.join(TRACES, "*.json")):
        h = hashlib.sha256(open(p, "rb").read()).hexdigest()
        if h in seen:
            continue
        seen.add(h)
        raw = open(p, "rb").read()
        try:
            w = json.loads(raw.decode().replace("\\", "\\\\"))
        except Exception:
            continue
        if w.get("nodes"):
            out.append(w)
    return out


def chosen_of(w):
    """Last ok node whose code stores 'answer'; decoded."""
    for n in reversed(w.get("nodes", [])):
        if n.get("ok") and "answer" in (n.get("code") or "").lower():
            return decode(n.get("code"))
    return None


def main():
    by_task = {}
    for w in worlds():
        by_task.setdefault(w.get("task_id", "?"), []).append(w)
    for ws in by_task.values():
        ws.sort(key=lambda w: w.get("ts", 0))

    pairs, seen_keys = [], set()

    def emit(task, prompt, chosen, rejected, src, ts):
        key = (task, hashlib.sha256(chosen.encode()).hexdigest(),
               hashlib.sha256(rejected.encode()).hexdigest())
        if key in seen_keys or chosen == rejected:
            return
        seen_keys.add(key)
        pairs.append({"task": task, "prompt": prompt, "chosen": chosen,
                      "rejected": rejected, "src": src, "ts": ts})

    for task, ws in by_task.items():
        # within-world
        for w in ws:
            if not w.get("completed"):
                continue
            ch = chosen_of(w)
            if not ch:
                continue
            for n in w.get("nodes", []):
                code = n.get("code") or ""
                if code and decode(code) != ch:
                    emit(task, w.get("task", ""), ch, decode(code),
                         "within", w.get("ts", 0))
        # cross-world: first completed world vs strictly earlier worlds
        for i, w in enumerate(ws):
            if w.get("completed"):
                ch = chosen_of(w)
                if not ch:
                    break
                for earlier in ws[:i]:
                    for n in earlier.get("nodes", []):
                        code = n.get("code") or ""
                        if code:
                            emit(task, w.get("task", ""), ch, decode(code),
                                 "cross", w.get("ts", 0))
                break

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w") as f:
        for p in pairs:
            f.write(json.dumps(p) + "\n")

    per = {}
    for p in pairs:
        per[p["task"]] = per.get(p["task"], 0) + 1
    n_within = sum(1 for p in pairs if p["src"] == "within")
    print(f"prefs: {len(pairs)} pairs ({n_within} within, "
          f"{len(pairs) - n_within} cross) → {OUT}")
    for t, n in sorted(per.items(), key=lambda kv: -kv[1]):
        print(f"  {t:16s} {n}")


if __name__ == "__main__":
    main()
