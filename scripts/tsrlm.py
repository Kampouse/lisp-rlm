#!/usr/bin/env python3
"""tsrlm — the same RLM loop, TypeScript surface instead of lisp-rlm.

Natural experiment (JP, 2026-10-01: "what would be funny is get it to do
typescript"): same brain (qwen3.5-4b via the local shim), same tasks,
same verification — only the language changes. If solve rates jump, the
Lisp surface was the bottleneck; if not, the brain is.

Simplified loop: prompt (TS sheet + task) → code → bun run → RLMANSWER
marker → verify → retry with error feedback (8 iters). No Q/CCG/dream
layers — this is the probe, not the product.
"""
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import urllib.request
import urllib.error

BASE = "http://127.0.0.1:8765/v1"
KEY = "local-shim"
MODEL = "qwen3.5-4b"
MAX_ITERS = 8

SHEET = """You write TypeScript (run with bun, TS strict-ish, no imports, no
external deps, no stdin). Numbers are plain JS numbers. CRITICAL: store
your final result by printing EXACTLY one line:
  console.log("RLMANSWER:" + JSON.stringify(<value>));
then stop. Output ONE fenced ```ts code block and nothing else.

ALLOWED: const/let, functions, arrow fns, if/else, for/for-of/while,
array methods (map/filter/reduce), Math.*, template literals,
Number.parseInt/toString. NO imports, NO process/fs, NO async, NO
classes needed.
"""

TASKS = {
    # same numbers as the lisp-rlm task suite
    "t1_sumsq": ("Compute the sum of squares of integers 1..10 (385). "
                 "Print RLMANSWER with the number.", lambda a: a == 385),
    "t3_fib": ("Compute the 15th Fibonacci number (1 1 2 3 ... f(15)=610, "
               "f(1)=f(2)=1). Print RLMANSWER with the number.",
               lambda a: a == 610),
    "t_lend_accrue": ("Accrue simple interest: principal=1000, rate=3 "
                      "per-10000 per block, blocks=200; new principal = "
                      "principal + Math.trunc(principal*rate*blocks/10000) "
                      "(= 1060). Print RLMANSWER with the integer.",
                      lambda a: a == 1060),
    "td_primeprod": ("Sum the primes below 20 (2+3+5+7+11+13+17+19 = 77), "
                     "then multiply that sum by 3 (231). Print RLMANSWER "
                     "with the integer.", lambda a: a == 231),
}


def llm(prompt, temp=0.3):
    body = json.dumps({
        "model": MODEL, "temperature": temp, "max_tokens": 1024,
        "messages": [{"role": "user", "content": prompt}],
    }).encode()
    last = None
    for attempt in range(15):
        req = urllib.request.Request(
            BASE + "/chat/completions", data=body,
            headers={"Authorization": f"Bearer {KEY}",
                     "Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=180) as r:
                return json.load(r)["choices"][0]["message"]["content"]
        except urllib.error.HTTPError as e:
            last = e
            if e.code in (429, 500, 503):
                time.sleep(min(30, 4 + attempt * 4))  # shim busy (flywheel)
                continue
            raise
    raise RuntimeError(f"shim saturated: {last}")


def extract_code(resp):
    m = re.search(r"```(?:ts|typescript)?\s*\n(.*?)```", resp, re.S)
    return m.group(1) if m else resp.strip()


def run_ts(code):
    with tempfile.NamedTemporaryFile("w", suffix=".ts", delete=False) as f:
        f.write(code)
        path = f.name
    try:
        p = subprocess.run(["bun", "run", path], capture_output=True,
                           text=True, timeout=30)
        out = (p.stdout or "") + (p.stderr or "")
        m = re.search(r"RLMANSWER:(.*)", out)
        ans = json.loads(m.group(1).strip()) if m else None
        return p.returncode, out[-1500:], ans
    except subprocess.TimeoutExpired:
        return 124, "TIMEOUT after 30s", None
    finally:
        os.unlink(path)


def episode(name, verify, n=2):
    solves = []
    for ep in range(n):
        task = TASKS[name][0]
        ctx = SHEET + "\nTASK:\n" + task
        solved, iters_used, last = False, 0, ""
        i = 0
        while i < MAX_ITERS:
            i += 1
            iters_used = i
            try:
                code = extract_code(llm(ctx))
            except Exception as e:
                i -= 1  # transport failure is not a task attempt
                time.sleep(5)
                continue
            rc, out, ans = run_ts(code)
            if ans is not None and verify(ans):
                solved = True
                break
            last = out.strip().splitlines()[-1] if out.strip() else "no output"
            ctx += (f"\n\nYOUR LAST CODE FAILED (rc={rc})."
                    f"\n```ts\n{code}\n```\n"
                    f"PROBLEM: {'wrong answer: ' + repr(ans) if ans is not None else out[-400:]} "
                    f"Fix and resubmit.")
        solves.append(solved)
        print(f"  {name} ep{ep+1}: {'SOLVED' if solved else 'FAIL'} "
              f"in {iters_used} iters (last: {last[:80]})", flush=True)
    return solves


if __name__ == "__main__":
    print(f"tsrlm probe — {MODEL} on TypeScript, {MAX_ITERS} iters max\n")
    results = {}
    for t in (sys.argv[1:] or TASKS):
        print(f"[{t}]", flush=True)
        results[t] = episode(t, TASKS[t][1])
    print("\n=== SUMMARY (tsrlm) ===")
    for t, s in results.items():
        print(f"{t}: {sum(s)}/{len(s)} episodes solved")
