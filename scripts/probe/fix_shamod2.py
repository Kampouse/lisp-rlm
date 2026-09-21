#!/usr/bin/env python3
"""fix_shamod2.py v2 — root-cause fixes for shamod.lisp + bip340.lisp.
Fix A: 6 LSB-first comparators -> MSB-first (fe-mul x2, fe-muln x2, fe-add, sc-addmod).
Fix B: 6 out-of-bounds (c-X 0) 9) reads -> drop the zero limb-9 term (keep borrow).
"""
import re, shutil

FILES = ['/tmp/nostr_probe/shamod.lisp', '/tmp/nostr_probe/bip340.lisp']

def msb_chain(pairs, const, indent):
    inner = '0'
    for var, cidx in reversed(pairs):
        vn = f'(vec-nth ({const} 0) {cidx})'
        inner = f'(if (> {var} {vn}) 1 (if (< {var} {vn}) 0 {inner}))'
    return f'{indent}(set! g {inner})'

re_t9 = re.compile(r'^\s*\(set! g \(if \(> t9 \(vec-nth \((c-[pn]+) 0\) 0\)\) 1 .*$')
re_d0 = re.compile(r'^\s*\(set! g \(if \(> d0 \(vec-nth \((c-[pn]+) 0\) 0\)\) 1 .*$')
re_oob_t18 = re.compile(r'\(set! dv \(\+ \(- \(- t18 \(vec-nth \((c-[pn]+) 0\) 9\)\) dbr\) 1073741824\)\)')
re_oob_d9 = re.compile(r'\(set! dv \(\+ \(- \(- d9 \(vec-nth \((c-[pn]+) 0\) 9\)\) dbr\) 1073741824\)\)')

REPL_T18 = '(set! dv (+ (- t18 dbr) 1073741824))'
REPL_D9 = '(set! dv (+ (- d9 dbr) 1073741824))'

for path in FILES:
    shutil.copy(path, path + '.pre-fix2')
    src = open(path).read()
    lines = src.split('\n')
    stats = {'t9': 0, 'd0': 0, 'oob_t18': 0, 'oob_d9': 0}
    for i, line in enumerate(lines):
        indent = line[:len(line) - len(line.lstrip())]
        m = re_t9.search(line)
        if m:
            pairs = [(f't{17 - k}', 8 - k) for k in range(9)]
            lines[i] = msb_chain(pairs, m.group(1), indent)
            stats['t9'] += 1
            continue
        m = re_d0.search(line)
        if m:
            pairs = [(f'd{j}', j) for j in range(8, -1, -1)]
            lines[i] = msb_chain(pairs, m.group(1), indent)
            stats['d0'] += 1
            continue
        if re_oob_t18.search(line):
            lines[i] = re_oob_t18.sub(REPL_T18, line)
            stats['oob_t18'] += 1
            continue
        if re_oob_d9.search(line):
            lines[i] = re_oob_d9.sub(REPL_D9, line)
            stats['oob_d9'] += 1
    out = '\n'.join(lines)
    d = 0
    instr = False
    for k, ch in enumerate(out):
        if ch == '"' and (k == 0 or out[k-1] != '\\'):
            instr = not instr
        elif not instr:
            if ch == '(':
                d += 1
            elif ch == ')':
                d -= 1
    open(path, 'w').write(out)
    print(f"{path}: {stats} paren-balance={d}")
