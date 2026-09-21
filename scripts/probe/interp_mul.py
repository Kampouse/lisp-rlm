#!/usr/bin/env python3
"""Mechanical interpreter for emitted shamod function bodies (fe-mul/fe-muln).
Parses the actual (define ...) text and executes it statement-by-statement,
so semantics are ground truth, not my re-derivation. Dumps state per row."""
import re, sys

src = open('/tmp/nostr_probe/shamod.lisp').read()

def extract_def(src, name):
    i = src.find(f"(define ({name} ")
    depth = 0
    j = i
    while j < len(src):
        if src[j] == '(':
            depth += 1
        elif src[j] == ')':
            depth -= 1
            if depth == 0:
                return src[i:j+1]
        j += 1

def tokenize(s):
    return s.replace('(', ' ( ').replace(')', ' ) ').split()

def parse(tokens):
    def rd(pos):
        tok = tokens[pos]
        if tok == '(':
            lst = []
            pos += 1
            while tokens[pos] != ')':
                sub, pos = rd(pos)
                lst.append(sub)
            return lst, pos + 1
        return tok, pos + 1
    ast, _ = rd(0)
    return ast

class Interp:
    def __init__(self, consts):
        self.consts = consts
        self.env = {}
        self.trace = []

    def ev(self, node):
        if isinstance(node, str):
            try:
                return int(node)
            except ValueError:
                if node in self.env:
                    return self.env[node]
                if node in self.consts:
                    return self.consts[node]
                raise NameError(node)
        hd = node[0]
        if hd in self.consts:
            return self.consts[hd]
        if hd == '+':
            return sum(self.ev(a) for a in node[1:])
        if hd == '-':
            return self.ev(node[1]) - sum(self.ev(a) for a in node[2:])
        if hd == 'band':
            v = self.ev(node[1])
            for a in node[2:]:
                v &= self.ev(a)
            return v
        if hd == 'shr':
            return self.ev(node[1]) >> self.ev(node[2])
        if hd == 'shl':
            return self.ev(node[1]) << self.ev(node[2])
        if hd == 'wrap-mul':
            return (self.ev(node[1]) * self.ev(node[2])) & ((1 << 60) - 1)
        if hd == 'vec-nth':
            vec = self.ev(node[1])
            idx = self.ev(node[2])
            if not isinstance(vec, list):
                raise TypeError(f"vec-nth on non-list {node[1]!r} = {vec!r}")
            if idx >= len(vec):
                raise IndexError(f"vec-nth idx {idx} len {len(vec)} in {node!r} (env near: c={self.env.get('c')}, t8={self.env.get('t8')})")
            return vec[idx]
        if hd == 'list':
            return [self.ev(a) for a in node[1:]]
        if hd == '=':
            return 1 if self.ev(node[1]) == self.ev(node[2]) else 0
        if hd == '!=':
            return 1 if self.ev(node[1]) != self.ev(node[2]) else 0
        if hd == '>':
            return 1 if self.ev(node[1]) > self.ev(node[2]) else 0
        if hd == '<':
            return 1 if self.ev(node[1]) < self.ev(node[2]) else 0
        if hd == 'and':
            return 1 if all(self.ev(a) for a in node[1:]) else 0
        if hd == 'if':
            return self.ev(node[2]) if self.ev(node[1]) else self.ev(node[3])
        if hd == 'begin':
            r = None
            for a in node[1:]:
                r = self.ev(a)
            return r
        if hd == 'set!':
            self.env[node[1]] = self.ev(node[2])
            return 0
        if hd == 'let':
            # (let ((x 0) ...) body...)
            for b in node[1]:
                self.env[b[0]] = self.ev(b[1])
            r = None
            for a in node[2:]:
                r = self.ev(a)
            return r
        if hd == 'let*':
            for b in node[1]:
                self.env[b[0]] = self.ev(b[1])
            r = None
            for a in node[2:]:
                r = self.ev(a)
            return r
        raise ValueError(f"unknown form {hd}")

M30 = (1 << 30) - 1

def run_fn(name, x, y, consts, dump_rows=False):
    body = extract_def(src, name)
    toks = tokenize(body)
    ast = parse(toks)
    # ast = ['define', ['name', params...], ['let', bindings, ['begin', stmts...]]]
    params = ast[1][1:]
    bindings = ast[2][1]
    begin = ast[2][2]
    it = Interp(consts)
    for b in bindings:
        it.env[b[0]] = it.ev(b[1])
    # bind params: x-limbs positional, y as list
    for pi, pname in enumerate(params[:-1]):
        it.env[pname] = x[pi]
    it.env[params[-1]] = y
    # execute begin statements; detect phase boundaries for dumping
    V = None
    row_markers = []
    stmts = begin[1:]
    for si, st in enumerate(stmts):
        it.ev(st)
        txt = ' '.join(tokenize(str(st)))[:60] if not isinstance(st, str) else st
        # dump state after each phase-3 row start: heuristic — statements touching (c-X 0) with m-var
    if dump_rows:
        # dump final t0..t18 as V
        tvals = [it.env[f't{i}'] for i in range(19)]
        V = sum(tvals[i] << (30 * i) for i in range(19))
        return it, V
    res = [it.env[f'd{i}'] for i in range(9)]
    return res, it

# final answer: the emitted fn returns (list d0..d8)
def full_run(name, x, y, consts, mod_limbs):
    body = extract_def(src, name)
    toks = tokenize(body)
    ast = parse(toks)
    params = ast[1][1:]
    it = Interp(consts)
    for b in ast[2][1]:
        it.env[b[0]] = it.ev(b[1])
    for pi, pname in enumerate(params[:-1]):
        it.env[pname] = x[pi]
    it.env[params[-1]] = y
    it.ev(ast[2][2])
    return [it.env[f'd{i}'] for i in range(9)]

def to30(v, k=9):
    return [(v >> (30 * i)) & M30 for i in range(k)]
def asm30(l):
    return sum(l[i] << (30 * i) for i in range(len(l)))

n = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
p = 2**256 - 2**32 - 977
R = 1 << 270

consts = {}
for m in re.finditer(r'\(define \(c-([a-z0-9]+) _d\) \(list ([^\)]*)\)\)', src):
    consts['c-' + m.group(1)] = [int(v) for v in m.group(2).split()]

if __name__ == '__main__':
    # Test: fe-muln interpreter vs algebra
    print("=== interpreter fe-muln (mod n) ===")
    for x, y in [(1, 1), (2, 1), (0xdeadbeef, 2)]:
        res = full_run('fe-muln', to30(x), to30(y), consts, to30(n))
        got = asm30(res)
        want = x * y * pow(R, -1, n) % n
        print(f"x={x} y={y}: {'OK' if got == want else 'WRONG delta=' + hex((got - want) % n)}")

    print("=== interpreter fe-mul (mod p) ===")
    for x, y in [(1, 1), (0xdeadbeef, 3)]:
        res = full_run('fe-mul', to30(x), to30(y), consts, to30(p))
        got = asm30(res)
        want = x * y * pow(R, -1, p) % p
        print(f"x={x} y={y}: {'OK' if got == want else 'WRONG delta=' + hex((got - want) % p)}")
