#!/usr/bin/env python3
"""gen-ts-twins — auto-generate <task>@ts.lisp twins from every base task.

The TS-vs-Lisp surface race needs identical curricula on both arms. This
stamps scripts/rlm-tasks/t*.lisp bases into @ts twins:

  - loads rlm_ts.lisp, sets __surface "ts"
  - __trace_id / RLMDUMP names get the @ts suffix (CCG memory isolation)
  - llm-code wraps through ts->lisp + strip-code-fences + build-prompt-ts
  - prompt lisp-isms rewritten (lisp code → TypeScript, (final true) →
    rlm_set("Final", true), (rlm-set X v) → rlm_set("X", v))

Skips (with a warning): tasks whose prompt requires (sub-rlm ...) —
delegation has no TS-surface spelling yet. Existing handcrafted twins are
never overwritten (pass --force to regenerate over them).

Verify functions pass through untouched — same runtime, same worlds,
only the brain's surface differs.
"""
import os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
TASKS = os.path.join(HERE, "rlm-tasks")

PROMPT_SWAPS = [
    (re.compile(r"by writing and evaluating lisp code", re.I),
     "by writing and evaluating TypeScript code"),
    (re.compile(r"\blisp code\b", re.I), "TypeScript code"),
    (re.compile(r"\bin lisp\b", re.I), "in TypeScript"),
    (re.compile(r"evaluate \(final true\)"), 'finish with rlm_set(\\"Final\\", true)'),
    (re.compile(r"\(rlm-set answer \.\.\.\)"), 'rlm_set(\\"answer\\", v)'),
    (re.compile(r"\(rlm-set Final true\)"), 'rlm_set(\\"Final\\", true)'),
    (re.compile(r"\(rlm-set Final t\)"), 'rlm_set(\\"Final\\", true)'),
]

def transform(src: str, name: str) -> str | None:
    m = re.search(r'\(run-rlm "((?:[^"\\]|\\.)*)"\)', src)
    if not m:
        print(f"  SKIP {name}: no (run-rlm ...) prompt found")
        return None
    prompt = m.group(1)
    if "sub-rlm" in prompt:
        print(f"  SKIP {name}: prompt requires (sub-rlm ...) — no TS spelling yet")
        return None

    out = src
    # 1. load rlm_ts.lisp after custom-actions
    out = out.replace(
        '(load-file "scripts/rlm-tasks/custom-actions.lisp")',
        '(load-file "scripts/rlm-tasks/custom-actions.lisp")\n'
        '(load-file "scripts/rlm-tasks/rlm_ts.lisp")',
    )
    # 2. surface flag after policy
    out = out.replace(
        "(rlm-set __policy POLICY_ID)",
        '(rlm-set __policy POLICY_ID)\n(rlm-set __surface "ts")',
    )
    # 3. trace_id + RLMDUMP task= get the @ts suffix
    out = re.sub(r'\(rlm-set __trace_id "%s"\)' % re.escape(name),
                 '(rlm-set __trace_id "%s@ts")' % name, out)
    out = out.replace(f'RLMDUMP task={name}")', f'RLMDUMP task={name}@ts")')
    # 4. llm-code through the TS surface
    out = out.replace(
        "(define (llm-code ctx)\n  (llm (build-prompt ctx)))",
        "(define (llm-code ctx)\n  (ts->lisp (strip-code-fences (llm (build-prompt-ts ctx)))))",
    )
    # 5. prompt de-lisping
    for rx, repl in PROMPT_SWAPS:
        out = rx.sub(lambda _m, r=repl: r.replace('\\\\', '\\'), out)
    # 6. header
    header = (";; AUTO-GENERATED @ts twin of %s.lisp — edit the BASE task and\n"
              ";; regenerate: python3 scripts/gen-ts-twins.py  (do not hand-edit)\n") % name
    return header + out

def main() -> None:
    force = "--force" in sys.argv
    made, skipped = 0, 0
    for f in sorted(os.listdir(TASKS)):
        if not (f.startswith("t") and f.endswith(".lisp")) or "@ts" in f:
            continue
        name = f[:-5]
        twin_path = os.path.join(TASKS, f"{name}@ts.lisp")
        if os.path.exists(twin_path) and not force:
            print(f"  keep  {name}@ts (exists, handcrafted or previously generated)")
            continue
        twin = transform(open(os.path.join(TASKS, f)).read(), name)
        if twin is None:
            skipped += 1
            continue
        open(twin_path, "w").write(twin)
        print(f"  wrote {name}@ts.lisp")
        made += 1
    print(f"done: {made} generated, {skipped} skipped")

if __name__ == "__main__":
    main()
