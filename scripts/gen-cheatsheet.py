#!/usr/bin/env python3
"""Regenerate data/rlm/dream/cheatsheet.txt from the kernel-true docs —
the ladder the solver sees in every prompt (exact names, exact arg order)."""
import os, re
REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
keep = re.compile(r"str|string|list|member|reverse|filter|map|reduce|sort|count|char|nth|index|length|append|slice|join|split", re.I)
src = os.path.join(REPO, "data/rlm/dream/docs.txt")
dst = os.path.join(REPO, "data/rlm/dream/cheatsheet.txt")
if os.path.exists(src):
    lines = [l for l in open(src) if l.strip() and keep.search(l)]
    open(dst, "w").write("AVAILABLE BUILTINS (exact names, exact arg order):\n" + "".join(lines))
    print(f"cheatsheet: {len(lines)} signatures")
