#!/usr/bin/env python3
"""wasm-fuzz.py — source-level differential fuzzer: interpreter vs WASM.

Complements the op-level fuzzer (tests/test_differential_fuzz.rs), which
diffs SpecVm ↔ Rust loop VM. That one never touches wasm; THIS one fuzzes
the surface that actually ships: random lisp programs compiled via
near-compile and executed via near-mock, trace-diffed against lisp-run.

Reuses trace-equiv.py's runners and classifier (same pipeline as the 33
hand probes, so classification semantics stay identical).

The generator is TYPED (int/str/list/bool): every form gets operands of
the right surface type, so programs compile on both surfaces and the
comparison exercises RUNTIME semantics (values, coercions, error paths)
rather than compile-stage rejections.

Usage:
  python3 scripts/wasm-fuzz.py [-n 50] [--seed 1] [--keep] [-v]
Exit code: 0 all agree (MATCH/BOTH_ERR_MATCH), 1 otherwise.
Repro any hit: python3 scripts/wasm-fuzz.py --seed <S> -n 1
"""
import argparse
import difflib
import importlib.util
import os
import random
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
_spec = importlib.util.spec_from_file_location(
    "trace_equiv", os.path.join(ROOT, "scripts", "trace-equiv.py"))
te = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(te)

T_INT, T_STR, T_LIST, T_BOOL, T_ANY = "int", "str", "list", "bool", "any"


class Gen:
    """Typed random lisp generator over the corpus vocabulary (e01–e35)."""

    STR_LITERALS = ["", "a", "ab", "hello world", "a:b:c", "XyZ", "0", "42",
                    "  pad  ", "near", "tx:1:lo", " Café", "ümlaut", "x\ty"]
    INT_LITERALS = [0, 1, 2, 3, 7, 10, 42, 99, 255, -1, -7, 1000,
                    1152921504606846975, 576460752303423488, -(2**59), 2**59 - 1]
    U128 = ["0", "1", "2", "999", "1000000000000000000",
            "340282366920938463463374607431768211455", "abc", ""]

    def __init__(self, rng: random.Random, seed: int):
        self.r = rng
        self.seed = seed
        self.vars = []        # (name, type)
        self.helpers = []     # (name, [(p, t)...], ret_t)

    # ── typed literals ───────────────────────────────────────────
    def lit_int(self):
        return str(self.r.choice(self.INT_LITERALS))

    def lit_str(self):
        v = self.r.choice(self.STR_LITERALS)
        v = v.replace("\\", "\\\\").replace('"', '\\"')
        v = v.replace("\n", "\\n").replace("\t", "\\t")
        return f'"{v}"'

    def lit_str_ne(self):
        """Non-empty string literal — str-replace pattern / str-split
        delimiter must be non-empty on the wasm surface (documented
        limitation: wasm refuses; interp inserts between chars)."""
        v = self.lit_str()
        while v in ('""',):
            v = self.lit_str()
        return v

    def lit_u128(self):
        return '"' + self.r.choice(self.U128) + '"'

    # ── typed expression generator ───────────────────────────────
    def expr(self, t, depth=3):
        if t == T_ANY:
            t = self.r.choice([T_INT, T_STR, T_LIST, T_BOOL])
        if depth <= 0:
            return self.leaf(t)

        int_forms = [
            "lit", "lit", "var",
            ("arith", 2), ("divmod", 2), ("abs", 1),
            ("len", 1),          # len of list
            ("str-length", 1),   # str-length of str
            ("str-index-of", 2),
            ("if", 3), ("let", 2), ("try", 2), ("call", 1),
        ]
        str_forms = [
            "lit", "lit", "var",
            ("str-cat", 2), ("str-join", 2),          # sep, list[str]
            ("str-map1", 1),                          # upcase/downcase/trim/reverse
            ("str-substring", 1), ("str-replace", 3),
            ("u128-arith", 2),
            ("if", 3), ("let", 2), ("try", 2), ("call", 1),
        ]
        list_forms = [
            "lit", "var",
            ("mklist", 1),     # homogeneous list of some element type
            ("cons", 2), ("cdr", 1), ("reverse-if", 1), ("split", 1), ("chunk", 1),
            # NB: no if/try here — branches would need ELEMENT-type agreement
            # ((list bool) vs (list str) unify-fail); interp is dynamic.
            ("let", 2),
        ]
        bool_forms = [
            "lit", "var",
            ("cmp", 2), ("logic", 2), ("not", 1),
            ("contains", 2), ("starts", 2), ("ends", 2),
            ("u128-cmp", 2),
            ("if", 3), ("let", 2), ("try", 2), ("call", 1),
        ]
        table = {T_INT: int_forms, T_STR: str_forms,
                 T_LIST: list_forms, T_BOOL: bool_forms}
        form = self.r.choice(table[t])

        if form == "lit":
            return self.leaf(t)
        if form == "var":
            cands = [(n, ty) for n, ty in self.vars if ty == t]
            return self.r.choice(cands)[0] if cands else self.leaf(t)
        name = form[0]
        d = depth - 1

        if name == "arith":
            op = self.r.choice(["+", "-", "*"])
            return f"({op} {self.expr(T_INT, d)} {self.expr(T_INT, d)})"
        if name == "divmod":
            op = self.r.choice(["/", "mod"])
            return f"({op} {self.expr(T_INT, d)} {self.expr(T_INT, d)})"
        if name == "abs":
            return f"(abs {self.expr(T_INT, d)})"
        if name == "len":
            return f"(len {self.expr(T_LIST, d)})"
        if name == "str-length":
            return f"(str-length {self.expr(T_STR, d)})"
        if name == "str-index-of":
            # wasm surface: needle must be a string literal
            return f"(str-index-of {self.expr(T_STR, d)} {self.lit_str()})"
        if name == "str-cat":
            return f"(str-cat {self.expr(T_STR, d)} {self.expr(T_STR, d)})"
        if name == "str-join":
            n = self.r.randint(0, 4)
            items = " ".join(self.expr(T_STR, d) for _ in range(n))
            return f"(str-join {self.lit_str()} (list {items}))"
            # NOTE: if the list happens to hold non-str elements this is a
            # runtime/type question — both surfaces see the same input.
        if name == "str-map1":
            op = self.r.choice(["str-upcase", "str-downcase", "str-trim"])
            return f"({op} {self.expr(T_STR, d)})"
        if name == "str-substring":
            a = self.r.randint(0, 8)
            b = self.r.randint(0, 8)
            return f"(str-substring {self.expr(T_STR, d)} {a} {b})"
        if name == "str-replace":
            # wasm surface: pattern must be a string literal
            return (f"(str-replace {self.expr(T_STR, d)} "
                    f"{self.lit_str_ne()} {self.lit_str()})")
        if name == "chunk":  # LIST-typed: str-chunk returns the chunks list
            n = self.r.randint(0, 6)
            return f"(str-chunk {self.expr(T_STR, d)} {n})"
        if name == "cdr":
            return f"(cdr {self.expr(T_LIST, d)})"
        if name == "split":  # LIST-typed: str-split returns the parts list
            # wasm surface: delimiter must be a string literal
            return f"(str-split {self.expr(T_STR, d)} {self.lit_str_ne()})"
        if name == "u128-arith":
            op = self.r.choice(["u128/add", "u128/sub", "u128/mul",
                                "u128/div", "u128/mod"])
            return f"({op} {self.lit_u128()} {self.lit_u128()})"
        if name == "u128-cmp":
            op = self.r.choice(["u128/gt", "u128/lt", "u128/eq"])
            return f"({op} {self.lit_u128()} {self.lit_u128()})"
        if name == "mklist":
            et = self.r.choice([T_INT, T_STR, T_BOOL])
            n = self.r.randint(0, 4)
            return "(list " + " ".join(self.expr(et, d) for _ in range(n)) + ")"
        if name == "cons":
            et = self.r.choice([T_INT, T_STR, T_BOOL])
            n = self.r.randint(0, 3)
            items = " ".join(self.leaf(et) for _ in range(n))
            return f"(cons {self.leaf(et)} (list {items}))"
        if name == "reverse-if":
            return f"(cdr {self.expr(T_LIST, d)})"
        if name == "cmp":
            # ordered comparisons are num-only on the wasm surface;
            # `=` is the polymorphic one (ints, strings, bools)
            if self.r.random() < 0.25:
                et = self.r.choice([T_INT, T_STR, T_BOOL])
                return f"(= {self.expr(et, d)} {self.expr(et, d)})"
            op = self.r.choice(["<", ">", "<=", ">="])
            return f"({op} {self.expr(T_INT, d)} {self.expr(T_INT, d)})"
        if name == "logic":
            op = self.r.choice(["and", "or"])
            return f"({op} {self.expr(T_BOOL, d)} {self.expr(T_BOOL, d)})"
        if name == "not":
            return f"(not {self.expr(T_BOOL, d)})"
        if name == "contains":
            return f"(str-contains {self.expr(T_STR, d)} {self.lit_str()})"
        # starts/ends: prefix/suffix must be literals on the wasm surface
        if name == "starts":
            return f"(str-starts-with {self.expr(T_STR, d)} {self.lit_str()})"
        if name == "ends":
            return f"(str-ends-with {self.expr(T_STR, d)} {self.lit_str()})"
        if name == "if":
            return f"(if {self.expr(T_BOOL, d)} {self.expr(t, d)} {self.expr(t, d)})"
        if name == "let":
            # canonical surface form: LIST OF PAIRS — (let ((v e)) body).
            # (Flat/vec single-binding `[v e]` is accepted by wasm_emit but
            # REJECTED by the interp compiler — surface asymmetry found by
            # this fuzzer on 2026-08-27; don't generate it.)
            n = f"v{len(self.vars)}{self.r.randint(0, 9)}"
            binding = self.expr(t, d)  # var NOT in scope for its own binding
            self.vars.append((n, t))
            try:
                return f"(let (({n} {binding})) {self.expr(t, d)})"
            finally:
                self.vars.pop()
        if name == "try":
            # handler result only — NEVER the catch-bound message itself
            # (its TEXT differs per surface by design; e34 note)
            return (f"(try {self.expr(t, d)} (catch e "
                    f"{self.r.choice(['\"caught\"', '0', 'nil']) if t == T_ANY else self.leaf(t)}))")
        if name == "call" and self.helpers:
            cands = [(nm, ps, rt) for nm, ps, rt in self.helpers if rt == t]
            if cands:
                nm, ps, rt = self.r.choice(cands)
                return "(" + nm + " " + " ".join(
                    self.expr(pt, d) for _, pt in ps) + ")"
            return self.leaf(t)
        return self.leaf(t)

    def leaf(self, t):
        if t == T_INT:
            return self.lit_int()
        if t == T_STR:
            return self.lit_str()
        if t == T_LIST:
            return "(list)"
        if t == T_BOOL:
            return self.r.choice(["true", "false"])
        return self.lit_int()

    # ── program assembly ─────────────────────────────────────────
    def program(self):
        self.vars = []
        self.helpers = []
        lines = [f";; wasm-fuzz seed={self.seed}"]
        for i in range(self.r.randint(0, 2)):
            name = f"helper{i}"
            arity = self.r.randint(1, 2)
            ptypes = [self.r.choice([T_INT, T_STR]) for _ in range(arity)]
            ret = self.r.choice([T_INT, T_STR, T_BOOL])
            ps = [(f"p{j}", ptypes[j]) for j in range(arity)]
            saved = self.vars
            self.vars = list(ps)
            body = self.expr(ret, 2)
            self.vars = saved
            self.helpers.append((name, ps, ret))
            sig = " ".join(p for p, _ in ps)
            lines.append(f"(define ({name} {sig}) {body})")
        lines.append("(define (main)")
        for _ in range(self.r.randint(2, 6)):
            lines.append(f"  (println {self.expr(T_ANY, 3)})")
        lines.append(")")
        return "\n".join(lines) + "\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("-n", type=int, default=50)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--keep", action="store_true")
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()

    counts = {}
    bad = 0
    for i in range(args.n):
        seed = args.seed + i
        g = Gen(random.Random(seed), seed)
        src = g.program()
        path = f"/tmp/wasm-fuzz-{seed}.lisp"
        with open(path, "w") as f:
            f.write(src)

        a_lines, a_err = te.interp_trace(path)
        w_lines, w_err = te.wasm_trace(path)
        cls, _ = te.classify(a_lines, a_err, w_lines, w_err)
        # Weak match: interp raises a precise error, wasm bare-traps
        # (`unreachable`, no message text). Both surfaces errored on the
        # same input — message-text equality is pinned by e34 probes, not
        # fuzzing. Counted separately so regressions are still visible.
        if cls == "BOTH_ERR_DIFF" and w_err and (
                "❌" in w_err or "trap" in w_err or "unreachable" in w_err):
            cls = "BOTH_ERR_TRAP"
        # Weak match: both refuse the same constant arithmetic overflow —
        # wasm folds at compile time, interp overflows at runtime. Same
        # semantics, different stage (pinned by e18/e20 probes).
        if (cls == "BOTH_ERR_DIFF" and a_err and w_err
                and (("overflow" in a_err and "overflow" in w_err)
                     or ("overflow" in a_err and "tagged range" in w_err))):
            cls = "BOTH_ERR_OVERFLOW"
        counts[cls] = counts.get(cls, 0) + 1
        ok = cls in ("MATCH", "BOTH_ERR_MATCH", "BOTH_ERR_TRAP", "BOTH_ERR_OVERFLOW")
        if not ok:
            bad += 1
            print(f"[{cls}] seed={seed}  repro: python3 scripts/wasm-fuzz.py --seed {seed} -n 1")
            if a_err or w_err:
                def _show(e):
                    e = e or ""
                    return e if len(e) <= 160 else e[:100] + " …[cut]… " + e[-120:]
                print(f"    interp: {_show(a_err)}\n    wasm:   {_show(w_err)}")
            else:
                d = list(difflib.unified_diff(a_lines, w_lines, "interp", "wasm",
                                              lineterm="", n=1))[:12]
                print("    " + "\n    ".join(d))
            if not args.keep:
                os.remove(path)
        elif args.verbose:
            print(f"[{cls}] seed={seed}")
        if ok and not args.keep:
            os.remove(path)

    print("\n== wasm-fuzz summary ==")
    for k in sorted(counts):
        print(f"  {k:16} {counts[k]}")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
