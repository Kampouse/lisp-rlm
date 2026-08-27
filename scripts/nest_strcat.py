#!/usr/bin/env python3
"""nest_strcat.py — rewrite >2-arg (str-cat a b c ...) to right-nested 2-arg.
The wasm emitter's str-cat is strict 2-ary; the interpreter is variadic.
Usage: python3 scripts/nest_strcat.py <file.lisp> [--check]
"""
import sys

def find_forms(s):
    out, i = [], 0
    while True:
        i = s.find("(str-cat", i)
        if i == -1:
            break
        depth, j, in_str = 0, i, False
        while j < len(s):
            c = s[j]
            if in_str:
                if c == '"' and s[j-1] != '\\':
                    in_str = False
            elif c == '"':
                in_str = True
            elif c == '(':
                depth += 1
            elif c == ')':
                depth -= 1
                if depth == 0:
                    break
            j += 1
        out.append((i, j + 1))
        i = j
    return out

def split_args(inner):
    args, cur, depth, in_str = [], "", 0, False
    for k, c in enumerate(inner):
        if in_str:
            cur += c
            if c == '"' and inner[k-1] != '\\':
                in_str = False
            continue
        if c == '"':
            in_str = True; cur += c; continue
        if c == '(':
            depth += 1; cur += c; continue
        if c == ')':
            depth -= 1; cur += c; continue
        if c == ' ' and depth == 0:
            if cur.strip():
                args.append(cur.strip())
            cur = ""
            continue
        cur += c
    if cur.strip():
        args.append(cur.strip())
    return args

def main():
    path = sys.argv[1]
    check = "--check" in sys.argv
    s = open(path).read()
    changed = 0
    while True:
        did = False
        for (a, b) in find_forms(s):
            args = split_args(s[a+1:b-1])
            if len(args) > 3:
                if check:
                    print(f"WOULD REWRITE line {s[:a].count(chr(10))+1}: {' '.join(args)[:80]}")
                    changed += 1
                    did = True
                    break
                parts = args[1:]
                nested = parts[-1]
                for p in reversed(parts[:-1]):
                    nested = f"(str-cat {p} {nested})"
                s = s[:a] + nested + s[b:]
                changed += 1
                did = True
                break
        if not did:
            break
    if check:
        print(f"{changed} forms need nesting in {path}")
        sys.exit(1 if changed else 0)
    open(path, "w").write(s)
    print(f"rewrote {changed} str-cat forms in {path}")

if __name__ == "__main__":
    main()
