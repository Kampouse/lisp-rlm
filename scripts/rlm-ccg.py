#!/usr/bin/env python3
"""CCG lane — Compact Context Graph over RLM trace nodes.

Implements the agent-memory half of the Compact Context Model (CCM)
paper, adapted to this harness: trace nodes are already structured
decisions (code/action/outcome), so no extraction LLM is needed for
the base layer. Deterministic edges:

  supersedes  — within one episode, attempt i+1 replaces attempt i
  solved-by   — within one task, the most recent success links back
                to the failure it plausibly fixed (same error class)
  similar     — same (error_class, failing_fn) disease across tasks

Output: data/rlm/ccg/graph.json (nodes+edges, incremental by trace
content hash) and data/rlm/ccg/<task>.txt — a compact "PAST ATTEMPTS"
memory block injected per-step by rlm-build-context (same pattern as
the cheatsheet). Corrections ("did you mean 'x'?") become first-class:
a failed attempt that hit a teaching error records the real function.
"""
import glob
import hashlib
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
TRACES = os.path.join(REPO, "data", "rlm", "traces")
CCG = os.path.join(REPO, "data", "rlm", "ccg")
LEDGER = os.path.join(REPO, "data", "rlm", "dream", "ledger.jsonl")
os.makedirs(CCG, exist_ok=True)

DYM = re.compile(r"did you mean '([^']+)'")
MAX_CODE = 70
MAX_OUT = 110


def load_state():
    p = os.path.join(CCG, "graph.json")
    if os.path.exists(p):
        return json.load(open(p))
    return {"nodes": {}, "edges": [], "processed": []}


def episodes():
    for p in sorted(glob.glob(os.path.join(TRACES, "*", "*.json")) +
                    glob.glob(os.path.join(TRACES, "*.json"))):
        raw = open(p, "rb").read()
        h = hashlib.sha256(raw).hexdigest()
        try:
            w = json.loads(raw.decode().replace("\\", "\\\\"))
        except Exception:
            continue
        nodes = w.get("nodes", [])
        if not nodes or not all("s" in n and "a" in n for n in nodes):
            continue
        yield h, w, nodes


def err_fields(s):
    """state key task|class|fn|rcN|esN|iN → (class, fn)"""
    parts = s.split("|")
    return (parts[1] if len(parts) > 1 else "", parts[2] if len(parts) > 2 else "")


def digest(code):
    c = (code or "").replace("\n", " ")
    return c[:MAX_CODE]


def build():
    st = load_state()
    done = set(st["processed"])
    new_nodes, new_edges = 0, 0
    for h, w, nodes in episodes():
        if h in done:
            continue
        done.add(h)
        tid = w.get("task_id", "anon")
        wid = str(w.get("ts", h[:8]))
        prev_nid = None
        for i, n in enumerate(nodes):
            nid = f"{tid}|{wid}|{i}"
            if nid in st["nodes"]:
                prev_nid = nid
                continue
            cls, fn = err_fields(n.get("s", ""))
            out = n.get("out", "") or ""
            dym = DYM.search(out)
            st["nodes"][nid] = {
                "task": tid, "i": i, "ok": bool(n.get("ok")),
                "act": n.get("a"), "cls": cls, "fn": fn,
                "code": digest(n.get("code", "")),
                "out": out[:MAX_OUT],
                "corr": f"use {dym.group(1)}" if dym else "",
                "ep_ok": bool(w.get("completed")),
            }
            new_nodes += 1
            # supersedes: next attempt replaces the previous code
            if prev_nid and st["nodes"].get(prev_nid, {}).get("code") != st["nodes"][nid]["code"]:
                st["edges"].append([prev_nid, nid, "supersedes"])
                new_edges += 1
            prev_nid = nid

    # solved-by: latest success per task ← most recent prior fail, same class
    by_task = {}
    for nid, nd in st["nodes"].items():
        by_task.setdefault(nd["task"], []).append((nid, nd))
    for tid, items in by_task.items():
        items.sort(key=lambda x: x[0])
        oks = [(nid, nd) for nid, nd in items if nd["ep_ok"]]
        if not oks:
            continue
        ok_nid, ok_nd = oks[-1]
        for nid, nd in reversed(items):
            if nid == ok_nid:
                break
            if not nd["ok"] and nd["cls"] == ok_nd["cls"]:
                if [nid, ok_nid, "solved-by"] not in st["edges"]:
                    st["edges"].append([nid, ok_nid, "solved-by"])
                    new_edges += 1
                break

    # similar: same disease (cls, fn) across different tasks
    disease = {}
    for nid, nd in st["nodes"].items():
        if nd["ok"] or not nd["fn"] or not nd["cls"]:
            continue
        disease.setdefault((nd["cls"], nd["fn"]), []).append(nid)
    for key, nids in disease.items():
        tasks = {st["nodes"][n]["task"] for n in nids}
        if len(tasks) > 1:
            a = nids[-1]
            for b in nids[-4:-1]:
                if st["nodes"][a]["task"] != st["nodes"][b]["task"]:
                    if [b, a, "similar"] not in st["edges"]:
                        st["edges"].append([b, a, "similar"])
                        new_edges += 1

    st["processed"] = sorted(done)[-4000:]
    json.dump(st, open(os.path.join(CCG, "graph.json"), "w"), indent=0)
    write_hints(by_task)
    with open(LEDGER, "a") as f:
        f.write(json.dumps({"event": "ccg", "ts": __import__("time").time(),
                            "nodes": len(st["nodes"]), "edges": len(st["edges"]),
                            "new_nodes": new_nodes, "new_edges": new_edges}) + "\n")
    print(f"ccg: {len(st['nodes'])} nodes, {len(st['edges'])} edges "
          f"(+{new_nodes}/+{new_edges}), hints for {len(by_task)} tasks")


def write_hints(by_task):
    for tid, items in by_task.items():
        items.sort(key=lambda x: x[0])
        fails = [(nid, nd) for nid, nd in items if not nd["ok"]][-3:]
        oks = [(nid, nd) for nid, nd in items if nd["ep_ok"]][-1:]
        lines = [f"### PAST ATTEMPTS ON {tid} (agent memory — do NOT repeat these failures)"]
        seen_code = set()
        for nid, nd in fails:
            if nd["code"] in seen_code:
                continue  # identical retry across episodes — one line is enough
            seen_code.add(nd["code"])
            corr = f" → CORRECTION: {nd['corr']}" if nd["corr"] else ""
            lines.append(f"- FAIL it{nd['i']}: {nd['code']} → {nd['out'][:60]}{corr}")
        for nid, nd in oks:
            lines.append(f"- SOLVED ONCE WITH: {nd['code']}")
        # cross-task same-disease corrections
        seen = set()
        for nid, nd in sorted(items, key=lambda x: x[0])[-6:]:
            if nd["corr"] and nd["corr"] not in seen:
                seen.add(nd["corr"])
        if len(seen) == 1:
            lines.append(f"- KNOWN TRAP on this task: {seen.pop()}")
        if len(lines) == 1:
            continue  # nothing useful yet
        open(os.path.join(CCG, f"{tid}.txt"), "w").write("\n".join(lines) + "\n")


if __name__ == "__main__":
    build()
