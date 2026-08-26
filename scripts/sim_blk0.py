#!/usr/bin/env python3
"""Interpret sha-blk0 from the built lisp file, compare per-round vs reference."""
import re, ast, sys

M = 0xFFFFFFFF
src = open('examples/bip340_smoke.lisp').read()
blk0 = src[src.index('(define (sha-blk0 W h0'):src.index('(define (sha-blk1 W h0')]

# ── tiny sexp parser ──
def parse(s):
    toks = re.findall(r'\(|\)|[^\s()]+', s)
    pos = 0
    def rd():
        nonlocal pos
        t = toks[pos]; pos += 1
        if t == '(':
            lst = []
            while toks[pos] != ')':
                lst.append(rd())
            pos += 1
            return lst
        if t.isdigit() or (t[0] == '-' and t[1:].isdigit()):
            return int(t)
        return t
    forms = []
    while pos < len(toks):
        forms.append(rd())
    return forms

forms = parse(blk0)
body = forms[0][2]  # (define (sha-blk0 W h0..h7) body) — [define, params, body]
# body: (let (bindings) (begin ... ))
bindings = body[1]

ROUND_SNAP = []
def ev(x, env):
    if isinstance(x, int):
        return x & M
    if isinstance(x, str):
        v = env[x]
        return v if isinstance(v, list) else v & M
    op = x[0]
    if op == 'band':
        r = M
        for a in x[1:]:
            r &= ev(a, env)
        return r & M
    if op == 'bor':
        r = 0
        for a in x[1:]:
            r |= ev(a, env)
        return r & M
    if op == 'shr':
        return (ev(x[1], env) >> ev(x[2], env)) & M
    if op == 'shl':
        return (ev(x[1], env) << ev(x[2], env)) & M
    if op == '+':
        r = 0
        for a in x[1:]:
            r += ev(a, env)
        return r & M
    if op == '-':
        r = ev(x[1], env)
        for a in x[2:]:
            r -= ev(a, env)
        return r & M
    if op == 'wrap-mul':
        r = 1
        for a in x[1:]:
            r *= ev(a, env)
        return r & M
    if op == 'vec-nth':
        return ev(x[1], env)[ev(x[2], env)] & M
    if op == 'set!':
        env[x[1]] = ev(x[2], env)
        if x[1] == 'a':
            ROUND_SNAP.append(tuple(env[k] & M for k in 'abcdefgh'))
        return None
    if op == 'begin':
        for a in x[1:]:
            ev(a, env)
        return None
    if op == 'let':
        for b in x[1]:
            env[b[0]] = ev(b[1], env)
        return ev(x[2], env)
    if op == 'vec-push':
        return ev(x[1], env) + [ev(x[2], env)]
    if op == 'list':
        return [ev(a, env) for a in x[1:]]
    raise Exception("op? %r" % (op,))

# ── reference ──
def rotr(x, n): return ((x >> n) | (x << (32 - n))) & M
def s0(x): return rotr(x,7) ^ rotr(x,18) ^ (x >> 3)
def s1(x): return rotr(x,17) ^ rotr(x,19) ^ (x >> 10)
W = [0x80000000] + [0]*14 + [0]
for i in range(16, 64):
    W.append((W[i-16] + s0(W[i-15]) + W[i-7] + s1(W[i-2])) & M)
K = [0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
     0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
     0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
     0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
     0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
     0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
     0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
     0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2]
def S1(x): return rotr(x,6) ^ rotr(x,11) ^ rotr(x,25)
def S0(x): return rotr(x,2) ^ rotr(x,13) ^ rotr(x,22)
def ch(e,f,g): return (e & f) ^ (~e & g)
def maj(a,b,c): return (a & b) ^ (a & c) ^ (b & c)
h = [0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19]
a,b,c,d,e,f,g,hh = h
ref_states = {}
for i in range(64):
    t1 = (hh + S1(e) + ch(e,f,g) + K[i] + W[i]) & M
    t2 = (S0(a) + maj(a,b,c)) & M
    hh,g,f = g,f,e
    e = (d + t1) & M
    d,c,b = c,b,a
    a = (t1 + t2) & M
    ref_states[i] = (a,b,c,d,e,f,g,hh)

# ── run lisp blk0 ──
env = {'W': W,
       'h0': h[0], 'h1': h[1], 'h2': h[2], 'h3': h[3],
       'h4': h[4], 'h5': h[5], 'h6': h[6], 'h7': h[7]}
ev(body, env)
got = tuple(env[k] & M for k in 'abcdefgh')
want = ref_states[63]
print("lisp final:", got)
print("reference :", want)
print("MATCH" if got == want else "MISMATCH")

# per-round divergence: ROUND_SNAP[r] = state after round r's (set! a ...)
for r in range(len(ROUND_SNAP)):
    if ROUND_SNAP[r] != ref_states[r]:
        print("first divergence at round", r)
        print(" lisp after round:", ROUND_SNAP[r])
        print(" ref  after round:", ref_states[r])
        if r > 0:
            print(" prev round matched:", ROUND_SNAP[r-1] == ref_states[r-1])
        break
else:
    print("all 64 rounds consistent")
