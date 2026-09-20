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
        ans = str(w.get("answer", "") or "")
        for k, v in (("~~NL~~", " "), ("~~QT~~", '"'), ("~~BS~~", "\\")):
            ans = ans.replace(k, v)
        ws.append({
            "ts": m2ts(hhmm), "task": task,
            "completed": bool(w.get("completed")),
            "iters": int(w.get("iterations", len(codes))),
            "distinct": len(set(codes)),
            "policy": w.get("policy", "?"),
            "lesson": w.get("lesson", ""),
            "answer": ans[:48],
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
            d["t"] = time.strftime("%H:%M", time.localtime(d.get("ts", 0)))
            deploys.append(d)

    corpus = 0
    cp = os.path.join(RLVR, "corpus.jsonl")
    if os.path.exists(cp):
        corpus = sum(1 for _ in open(cp))

    # ---- task board: per-task stats, last answer, partial credit, exemplar ----
    import importlib.util as _ilu
    _sp = _ilu.spec_from_file_location("rq", os.path.join(REPO, "scripts", "rlm-qlearn.py"))
    _rq = _ilu.module_from_spec(_sp)
    try:
        _sp.loader.exec_module(_rq)
        _expected, _partial = _rq.EXPECTED, _rq.partial_score
    except Exception:
        _expected, _partial = {}, (lambda a, e: 0.0)

    EXDIR = os.path.join(REPO, "scripts", "rlm-tasks", "exemplars")
    by_task = defaultdict(list)
    for w in ws:
        by_task[w["task"]].append(w)
    tasks = []
    for task, grp in by_task.items():
        last = grp[-1]
        ex = "none"
        exp = os.path.join(EXDIR, task + ".txt")
        if os.path.exists(exp):
            head = open(exp).read(32)
            ex = "hand" if head.startswith("# hand") else "auto"
        exp_ans = _expected.get(task)
        part = None
        if not last["completed"] and exp_ans and last["answer"]:
            part = round(_partial(last["answer"], exp_ans), 2)
        tasks.append({
            "task": task, "n": len(grp),
            "solved": sum(1 for g in grp if g["completed"]),
            "avg_iters": round(sum(g["iters"] for g in grp) / len(grp), 1),
            "last_answer": last["answer"], "last_ok": last["completed"],
            "partial": part, "exemplar": ex,
            "expected": str(exp_ans)[:24] if exp_ans is not None else "",
        })
    tasks.sort(key=lambda t: (t["solved"] / t["n"], -t["n"]))

    # ---- grammar variant lane (g3 family: -g3 full / -g3l lean / -g3s strings-first) ----
    def variant_of(policy):
        p = str(policy)
        for name, suf in (("strings-first", "-g3s"), ("lean", "-g3l"), ("full", "-g3")):
            if p.endswith(suf):
                return name
        return None
    vstat = defaultdict(lambda: {"n": 0, "completed": 0, "iters": []})
    for w in ws:
        v = variant_of(w["policy"])
        if v:
            vstat[v]["n"] += 1
            vstat[v]["completed"] += 1 if w["completed"] else 0
            vstat[v]["iters"].append(w["iters"])
    variants = {v: {"n": b["n"], "completed": b["completed"],
                    "avg_iters": round(sum(b["iters"]) / len(b["iters"]), 1)}
                for v, b in vstat.items()}
    cur_variant = "full"
    vf = os.path.join(REPO, "scripts", "rlm-tasks", "grammar-variant.json")
    if os.path.exists(vf):
        try:
            cur_variant = json.load(open(vf)).get("variant", "full")
        except Exception:
            pass

    # ---- DPO preference pairs + worst Q-states ----
    prefs = 0
    pp = os.path.join(REPO, "data", "rlm", "prefs", "pairs.jsonl")
    if os.path.exists(pp):
        prefs = sum(1 for _ in open(pp))
    worst = []
    wp = os.path.join(REPO, "data", "rlm", "dream", "worst-states.txt")
    if os.path.exists(wp):
        worst = [l.strip() for l in open(wp) if l.strip()][:5]

    data = {
        "updated": time.strftime("%Y-%m-%d %H:%M:%S"),
        "worlds": len(ws),
        "corpus_rows": corpus,
        "deploys": deploys,
        "policies": policies,
        "rlm_cycles": rlm_cycles(ws),
        "rlvr_cycles": rlvr_cycles(),
        "latest": ws[-6:],
        "lessons": [w for w in ws if w.get("lesson")][-8:],
        "tasks": tasks,
        "variants": variants,
        "cur_variant": cur_variant,
        "prefs": prefs,
        "worst_states": worst,
    }
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    json.dump(data, open(OUT, "w"))
    print(f"dashboard: {len(ws)} worlds, {corpus} corpus rows, "
          f"{len(deploys)} ledger events")


if __name__ == "__main__":
    main()
