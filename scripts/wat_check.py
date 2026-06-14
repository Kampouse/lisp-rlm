#!/usr/bin/env python3
"""
Post-build WAT analyzer for lisp-rlm WASM contracts.

Checks:
1. View exports calling storage_write/storage_remove (silent trap on NEAR)
2. Host calls with suspicious arg patterns (missing untag on gas/index params)
3. i64.store stack order (addr before value, not reversed)
4. storage_write key_ptr targeting FP_GLOBAL instead of KEY_BUF
5. Unknown memory address constants (catches drifted offsets)
6. Depth counter read without matching write-back

Usage:
    python3 scripts/wat_check.py <file.wat|file.wasm> [--verbose]

Exit 0 = pass, 1 = issues found, 2 = usage error.
"""

import sys
import re
import os
import subprocess
from typing import List

KNOWN_BUFFERS = {
    56: "RUNTIME_HEAP_PTR", 64: "TEMP_MEM", 256: "AMOUNT_MEM",
    512: "KEY_BUF", 8192: "STORAGE_BUF", 16384: "INPUT_BUF",
    24576: "JSON_NEST_BUF", 32768: "STRING_BUILDER",
    36864: "BORSH_BUF", 106496: "FP_GLOBAL",
    131072: "PROMISE_RESULT_BUF", 999980: "DEPTH_COUNTER",
}


class ImportInfo:
    __slots__ = ("name", "func_idx", "line")
    def __init__(self, name, func_idx, line):
        self.name = name; self.func_idx = func_idx; self.line = line


class FuncInfo:
    __slots__ = ("idx", "name", "instructions", "start_line", "end_line")
    def __init__(self, idx, name, start_line):
        self.idx = idx; self.name = name; self.instructions = []
        self.start_line = start_line; self.end_line = start_line


class ExportInfo:
    __slots__ = ("name", "func_idx", "line")
    def __init__(self, name, func_idx, line):
        self.name = name; self.func_idx = func_idx; self.line = line


class Issue:
    __slots__ = ("severity", "rule", "line", "func_idx", "func_name", "msg")
    def __init__(self, severity, rule, line, func_idx, func_name, msg):
        self.severity = severity; self.rule = rule; self.line = line
        self.func_idx = func_idx; self.func_name = func_name; self.msg = msg


def _count_parens(s):
    """Count parens in a string, ignoring those inside double quotes."""
    opens = closes = 0
    in_str = False
    for ch in s:
        if ch == '"':
            in_str = not in_str
        elif not in_str:
            if ch == '(':
                opens += 1
            elif ch == ')':
                closes += 1
    return opens, closes


def parse_wat(text):
    """Parse WAT into imports, functions, exports."""
    lines = text.splitlines()
    imports, functions, exports = [], [], []

    depth = 0
    form_start = None
    form_text = []

    for lineno_0, line in enumerate(lines):
        lineno = lineno_0 + 1
        stripped = line.strip()
        if not stripped:
            continue

        opens, closes = _count_parens(stripped)
        old_depth = depth
        depth += opens - closes

        # Skip (module opening line
        if old_depth == 0 and opens > 0 and '(module' in stripped:
            continue

        # Single-line form at depth 1 (opens==closes, balanced on one line)
        if old_depth == 1 and depth == 1 and opens > 0 and opens == closes:
            _parse_form(stripped, lineno, imports, functions, exports)
            continue

        # Multi-line form starts: depth goes from 1 to 2+
        if old_depth == 1 and opens > closes and depth >= 2:
            form_start = lineno
            form_text = [line]
            continue

        # Inside multi-line form
        if form_start is not None and depth >= 2:
            form_text.append(line)
            continue

        # Multi-line form ends: depth returns to 1
        if form_start is not None and old_depth == 2 and depth == 1:
            form_text.append(line)
            _parse_form('\n'.join(form_text), form_start, imports, functions, exports)
            form_start = None
            form_text = []
            continue

    return imports, functions, exports


def _parse_form(form_text, form_line, imports, functions, exports):
    """Parse one top-level module form."""
    stripped = form_text.strip()

    # Import: (import "env" "name" (func (;idx;) (type N)))
    # With (;idx;) comments present or stripped
    m = re.match(r'\(import\s+"env"\s+"([^"]+)"\s+\(func\s+.*?\)\)', stripped)
    if m:
        idx = len(imports)
        imports.append(ImportInfo(m.group(1), idx, form_line))
        return

    # Export: (export "name" (func N))
    m = re.match(r'\(export\s+"([^"]+)"\s+\(func\s+(\d+)\)\)', stripped)
    if m:
        exports.append(ExportInfo(m.group(1), int(m.group(2)), form_line))
        return

    # Function: (func (;idx;) (type N) ...) or (func $name (;idx;) (type N) ...)
    m = re.match(r'\(func\s+(?:\$\w+\s+)?\(;(\d+);\)', stripped)
    if not m:
        m = re.match(r'\(func\s+(?:\$\w+\s+)?\(type\s+\d+\)', stripped)
    if m:
        idx = len(imports) + len(functions)
        name_m = re.search(r'\$(\w[\w-]*)', stripped)
        name = name_m.group(1) if name_m else None
        fn = FuncInfo(idx, name, form_line)

        for bline in form_text.splitlines():
            bst = bline.strip()
            if not bst:
                continue
            if bst.startswith('(func') or bst.startswith('(type') or \
               bst.startswith('(param') or bst.startswith('(result') or \
               bst.startswith('(local'):
                continue
            fn.instructions.append(bst)

        for i in range(len(fn.instructions)):
            fn.instructions[i] = (form_line + i, fn.instructions[i])
        if fn.instructions:
            fn.end_line = fn.instructions[-1][0]
        functions.append(fn)


def _host(imports, idx):
    for imp in imports:
        if imp.func_idx == idx:
            return imp.name
    return None


# ── Checks ──────────────────────────────────────────────────────────────

def check_all(functions, exports, imports):
    issues = []

    # Build transitive call graph: func_idx -> set of (direct host calls)
    # And reverse: func_idx -> set of (func_idxs that call it)
    func_host_calls = {}  # func_idx -> set of host_names
    func_callers = {}     # func_idx -> set of caller func_idxs
    for fn in functions:
        direct_hosts = set()
        direct_calls = set()
        for i, (ln, inst) in enumerate(fn.instructions):
            cm = re.match(r'call\s+(\d+)', inst)
            if cm:
                cidx = int(cm.group(1))
                hname = _host(imports, cidx)
                if hname:
                    direct_hosts.add(hname)
                else:
                    direct_calls.add(cidx)
        func_host_calls[fn.idx] = direct_hosts
        for called_fn_idx in direct_calls:
            func_callers.setdefault(called_fn_idx, set()).add(fn.idx)

    # Compute transitive host calls for each function (BFS through call graph)
    def transitive_hosts(fn_idx, visited=None):
        if visited is None:
            visited = set()
        if fn_idx in visited:
            return set()
        visited.add(fn_idx)
        result = set(func_host_calls.get(fn_idx, set()))
        for callee in func_callers.get(fn_idx, set()):
            # callee is called BY fn_idx, not calling fn_idx
            pass
        # Find what fn_idx calls (including other internal funcs)
        fn_obj = None
        for f in functions:
            if f.idx == fn_idx:
                fn_obj = f
                break
        if fn_obj:
            for _, inst in fn_obj.instructions:
                cm = re.match(r'call\s+(\d+)', inst)
                if cm:
                    cidx = int(cm.group(1))
                    if not _host(imports, cidx):
                        result |= transitive_hosts(cidx, visited)
        return result

    for fn in functions:
        # Collect host calls in this function (direct only for other checks)
        host_calls = []  # [(instr_index, host_name, host_idx), ...]
        for i, (ln, inst) in enumerate(fn.instructions):
            cm = re.match(r'call\s+(\d+)', inst)
            if cm:
                cidx = int(cm.group(1))
                hname = _host(imports, cidx)
                if hname:
                    host_calls.append((i, hname, cidx))

        host_names = {hc[1] for hc in host_calls}
        is_exported = any(e.func_idx == fn.idx for e in exports)

        # Transitive host calls (includes calls through internal funcs)
        all_hosts = transitive_hosts(fn.idx)

        # CHECK 1: View mutation (export + storage_write in transitive closure + value_return)
        if is_exported and "storage_write" in all_hosts and "value_return" in all_hosts:
            issues.append(Issue("WARN", "VIEW_MUTATION", fn.start_line, fn.idx,
                                fn.name or f"func_{fn.idx}",
                                "Exported function calls storage_write AND value_return. "
                                "Views that write silently trap on NEAR."))

        # CHECK 2: Missing untag on sensitive host params
        for ci, (ii, hname, cidx) in enumerate(host_calls):
            need_untag = hname in ("promise_batch_action_transfer",
                                   "promise_result", "promise_batch_then")
            if not need_untag:
                continue
            has_shr = any("i64.shr_u" in fn.instructions[j][1]
                         for j in range(max(0, ii - 15), ii))
            if not has_shr:
                param = "idx" if hname != "promise_batch_then" else "gas"
                issues.append(Issue("ERROR", "MISSING_UNTAG", ii, fn.idx,
                                    fn.name or f"func_{fn.idx}",
                                    f"{hname}: {param} not untagged (no i64.shr_u). "
                                    f"Host receives garbage (8x real)."))

        # CHECK 3: storage_write with FP_GLOBAL as key_ptr
        for ci, (si, hname, cidx) in enumerate(host_calls):
            if hname != "storage_write":
                continue
            for j in range(max(0, si - 15), si):
                pm = re.match(r'i64\.const\s+(\d+)', fn.instructions[j][1])
                if pm and int(pm.group(1)) == 106496:
                    issues.append(Issue("WARN", "STORAGE_FPGLOBAL", si, fn.idx,
                                        fn.name or f"func_{fn.idx}",
                                        "storage_write key at FP_GLOBAL (106496). "
                                        "Use near/kstore (KEY_BUF=512)."))

        # CHECK 4: i64.store stack order
        for i, (ln, inst) in enumerate(fn.instructions):
            if not inst.startswith("i64.store"):
                continue
            if i < 2:
                continue
            p1 = fn.instructions[i - 1][1]  # top of stack = value (i64)
            p2 = fn.instructions[i - 2][1]  # below = addr (i32)
            # WRONG: i32 producer on top, i64 producer below
            if re.match(r'i32\.\w+', p1) and re.match(r'i64\.(const|load|extend)', p2):
                issues.append(Issue("ERROR", "STORE_ORDER", ln, fn.idx,
                                    fn.name or f"func_{fn.idx}",
                                    f"i64.store order: {p2.strip()} then {p1.strip()}. "
                                    f"Expects [addr:i32, value:i64]."))

        # CHECK 5: Unknown memory addresses
        seen = set()
        for i, (ln, inst) in enumerate(fn.instructions):
            m = re.match(r'i64\.const\s+(\d+)', inst)
            if not m:
                continue
            val = int(m.group(1))
            if val < 50 or val > 500000 or val in KNOWN_BUFFERS or val in seen:
                continue
            for j in range(i + 1, min(i + 5, len(fn.instructions))):
                nxt = fn.instructions[j][1]
                if any(op in nxt for op in ("i64.store", "i64.load", "i32.store8", "i32.load8_u")):
                    seen.add(val)
                    issues.append(Issue("WARN", "UNKNOWN_ADDR", ln, fn.idx,
                                        fn.name or f"func_{fn.idx}",
                                        f"Unknown mem addr {val} near store/load."))
                    break

        # CHECK 6: Depth counter read without write-back
        reads = writes = False
        for _, inst in fn.instructions:
            if "999980" not in inst:
                continue
            if "i64.load" in inst:
                reads = True
            if "i64.store" in inst:
                writes = True
        if reads and not writes:
            issues.append(Issue("WARN", "DEPTH_COUNTER", fn.start_line, fn.idx,
                                fn.name or f"func_{fn.idx}",
                                "Reads DEPTH_COUNTER (999980) without write-back."))

    return issues


def print_report(issues, verbose=False):
    errors = [i for i in issues if i.severity == "ERROR"]
    warns = [i for i in issues if i.severity == "WARN"]
    if not issues:
        print("PASS: 0 issues found")
        return 0
    print(f"FOUND: {len(errors)} error(s), {len(warns)} warning(s)\n")
    for iss in sorted(issues, key=lambda x: (0 if x.severity == "ERROR" else 1, x.line)):
        tag = "E" if iss.severity == "ERROR" else "W"
        print(f"  [{tag}] {iss.rule} (L{iss.line}, {iss.func_name or iss.func_idx})")
        print(f"      {iss.msg}")
        if verbose:
            print()
    return 1 if errors else 0


def main():
    if len(sys.argv) < 2:
        print("Usage: wat_check.py <file.wat|file.wasm> [--verbose]", file=sys.stderr)
        sys.exit(2)

    path = sys.argv[1]
    verbose = "--verbose" in sys.argv or "-v" in sys.argv

    if path.endswith(".wasm"):
        wat_path = "/tmp/_wat_check_auto.wat"
        try:
            subprocess.run(["wasm2wat", path, "-o", wat_path], check=True, capture_output=True)
        except (subprocess.CalledProcessError, FileNotFoundError) as e:
            print(f"ERROR: wasm2wat: {e}", file=sys.stderr)
            sys.exit(2)
        path = wat_path

    with open(path) as f:
        wat_text = f.read()

    imports, functions, exports = parse_wat(wat_text)
    issues = check_all(functions, exports, imports)
    sys.exit(print_report(issues, verbose))


if __name__ == "__main__":
    main()
