#!/usr/bin/env python3
"""rlm-dashboard — regenerate dashboard/data.json from all recorded history.

Reads: trace worlds (data/rlm/traces/**), dream ledger, rlvr runs.jsonl +
corpus.jsonl. Emits one JSON the static page fetches. Called by the flywheel
after the dream step, so charts update every cycle. Pure reads, no LLM.
"""
import json, os, glob, re, time
from collections import defaultdict

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TRACES = os.path.join(REPO, "data", "rlm", "traces")
LEDGER = os.path.join(REPO, "data", "rlm", "dream", "ledger.jsonl")
RLVR = "/Users/asil/.openclaw/workspace/data/rlvr"
OUT = os.path.join(REPO, "dashboard", "data.json")


def m2ts(hhmmss):
    hhmmss = hhmmss[:2] + ":" + hhmmss[2:4] + ":" + hhmmss[4:6]
    t = time.strptime(time.strftime("%Y-%m-%d ") + hhmmss, "%Y-%m-%d %H:%M:%S")
    return time.mktime(t)


def load_worlds():
    ws = []
    for p in glob.glob(os.path.join(TRACES, "*", "*.json")) + \
             glob.glob(os.path.join(TRACES, "*.json")):
        base = os.path.basename(p)
        m = re.match(r"(.+)-(\d{6})(-.+)?\.json", base)
        if not m:
            continue
        try:
            w = json.loads(open(p).read().replace("\\", "\\\\"))
        except Exception:
            continue
        task, hhmm = m.group(1), m.group(2)
        codes = [n.get("code", "") for n in w.get("nodes", [])]
        ws.append({
            "ts": m2ts(hhmm), "task": task,
            "completed": bool(w.get("completed")),
            "iters": int(w.get("iterations", len(codes))),
            "distinct": len(set(codes)),
            "policy": w.get("policy", "?"),
        })
    ws.sort(key=lambda x: x["ts"])
    return ws


def rlm_cycles(ws):
    """Group worlds into cycles (same-minute bucket)."""
    cyc = defaultdict(list)
    for w in ws:
        cyc[time.strftime("%H:%M", time.localtime(w["ts"]))].append(w)
    out = []
    for minute in sorted(cyc):
        grp = cyc[minute]
        out.append({
            "t": minute,
            "ts": grp[0]["ts"],
            "solved": sum(1 for g in grp if g["completed"]),
            "total": len(grp),
            "iters": round(sum(g["iters"] for g in grp) / len(grp), 1),
            "policies": sorted(set(g["policy"] for g in grp)),
        })
    return out


def rlvr_cycles():
    runs_p = os.path.join(RLVR, "runs.jsonl")
    out, by_min = [], defaultdict(lambda: [0, 0])
    if not os.path.exists(runs_p):
        return out
    for line in open(runs_p):
        try:
            r = json.loads(line)
        except Exception:
            continue
        key = time.strftime("%H:%M", time.localtime(r.get("ts", 0)))
        if r.get("reward", 0) > 0:
            by_min[key][0] += 1
        by_min[key][1] += 1
    for minute in sorted(by_min):
        ok, tot = by_min[minute]
        out.append({"t": minute, "pass": ok, "total": tot,
                    "rate": round(ok / tot, 3) if tot else 0})
    return out


def main():
    ws = load_worlds()
    by_policy = defaultdict(lambda: {"n": 0, "completed": 0, "iters": []})
    for w in ws:
        b = by_policy[w["policy"]]
        b["n"] += 1
        b["completed"] += 1 if w["completed"] else 0
        b["iters"].append(w["iters"])
    policies = {p: {"n": b["n"], "completed": b["completed"],
                    "avg_iters": round(sum(b["iters"]) / len(b["iters"]), 1)}
                for p, b in by_policy.items()}

    deploys = []
    if os.path.exists(LEDGER):
        for line in open(LEDGER):
            try:
                d = json.loads(line)
            except Exception:
                continue
            d["t"] = time.strftime("%H:%M", time.localtime(d["ts"]))
            deploys.append(d)

    corpus = 0
    cp = os.path.join(RLVR, "corpus.jsonl")
    if os.path.exists(cp):
        corpus = sum(1 for _ in open(cp))

    data = {
        "updated": time.strftime("%Y-%m-%d %H:%M:%S"),
        "worlds": len(ws),
        "corpus_rows": corpus,
        "deploys": deploys,
        "policies": policies,
        "rlm_cycles": rlm_cycles(ws),
        "rlvr_cycles": rlvr_cycles(),
        "latest": ws[-6:],
    }
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    json.dump(data, open(OUT, "w"))
    print(f"dashboard: {len(ws)} worlds, {corpus} corpus rows, "
          f"{len(deploys)} ledger events")


if __name__ == "__main__":
    main()
