#!/usr/bin/env python3
"""Generate data/rlm/ccg/known-fns.txt — every callable name, from the
kernel's own canonical table (src/helpers.rs builtin list), dispatch arms
as backup, kernel specials, docs.txt supplement, and (define (fn ...) in
rlm_runtime.lisp (runtime-defined helpers like parse-fix).

One name per line; consumed by rlm_runtime's pre-flight phantom lint.
"""
import os, re

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
STR = re.compile(r'"([a-zA-Z][a-zA-Z0-9!?*<>=+./-]*)"')
ARM = re.compile(r'"([a-zA-Z][a-zA-Z0-9!?*<>=+-]*)"\s*=>')
DEF = re.compile(r"\(define\s+\(([a-zA-Z][a-zA-Z0-9!?*<>=+-]*)")
SPECIALS = {
    "define", "lambda", "let", "if", "begin", "quote", "quasiquote",
    "unquote", "unquote-splicing", "set!", "try", "catch", "and", "or",
    "cond", "else", "when", "unless", "let*", "recur", "defmacro",
    "dotimes", "while", "for", "match", "final",
}

names = set(SPECIALS)

# 1) canonical table in helpers.rs (grab its list region wholesale)
src = open(os.path.join(REPO, "src", "helpers.rs"), encoding="utf8", errors="ignore").read()
names.update(STR.findall(src))

# 2) dispatch arms anywhere in src/
for root, _, files in os.walk(os.path.join(REPO, "src")):
    for f in files:
        if f.endswith(".rs"):
            names.update(ARM.findall(open(os.path.join(root, f),
                        encoding="utf8", errors="ignore").read()))

# 3) runtime-lisp defines (parse-fix, q-choose, ...)
rt = os.path.join(REPO, "rlm_runtime.lisp")
if os.path.exists(rt):
    names.update(DEF.findall(open(rt, encoding="utf8").read()))

# 4) docs.txt supplement
docs = os.path.join(REPO, "data", "rlm", "dream", "docs.txt")
if os.path.exists(docs):
    for line in open(docs, encoding="utf8"):
        line = line.strip()
        if line.startswith("(") and len(line) > 1 and line[1:].split():
            names.add(line[1:].split()[0].rstrip(")"))

out = os.path.join(REPO, "data", "rlm", "ccg", "known-fns.txt")
os.makedirs(os.path.dirname(out), exist_ok=True)
with open(out, "w") as fh:
    fh.write("\n".join(sorted(names)) + "\n")
print(f"known-fns: {len(names)} names")
