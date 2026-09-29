#!/usr/bin/env python3
"""fix_verify2.py — rep-domain rewrite of the verify appendix binds.
- fe-eqc: canonical equality via fe-words-pure (fm . c-onep), 8-word compare
- parity: bit0 of pure-word 0 (LSB-first)
- ladder: P/G as rep coords with Z=rep(1); acc=inf=(0,0,0); pt-dbl 3-ary
"""
import re, ast

P = '/tmp/nostr_probe/gen_bip340_verify.py'
s = open(P).read()

# 1) replace fe-eqn definition with canonical fe-eqc
s = re.sub(r"\(define \(fe-eqn A B\).*?\)\)\n",
           """(define (fe-eqc A B)
  (let ((wa (fe-words-pure (fm A (c-onep 0))))
        (wb (fe-words-pure (fm B (c-onep 0)))))
    (if (= (vec-nth wa 0) (vec-nth wb 0))
      (if (= (vec-nth wa 1) (vec-nth wb 1))
        (if (= (vec-nth wa 2) (vec-nth wb 2))
          (if (= (vec-nth wa 3) (vec-nth wb 3))
            (if (= (vec-nth wa 4) (vec-nth wb 4))
              (if (= (vec-nth wa 5) (vec-nth wb 5))
                (if (= (vec-nth wa 6) (vec-nth wb 6))
                  (if (= (vec-nth wa 7) (vec-nth wb 7)) 1 0)
                  0)
                0)
              0)
            0)
          0)
        0)
      0)))
""",
           s, flags=re.S)

# 2) all fe-eqn call sites -> fe-eqc
s = s.replace('(fe-eqn ', '(fe-eqc ')

# 3) parity reads: pure words are LSB-first -> word 0 bit 0
s = s.replace('(band (vec-nth (fm nyp (c-onep 0)) 7) 1)',
              '(band (vec-nth (fe-words-pure (fm nyp (c-onep 0))) 0) 1)')
s = s.replace('(band (vec-nth (fm nyr (c-onep 0)) 7) 1)',
              '(band (vec-nth (fe-words-pure (fm nyr (c-onep 0))) 0) 1)')

# 4) ladder points: rep coords, Z=rep(1); drop PLAIN/demont variants
s = s.replace("    ('pxP', '(fm pxm (c-onep 0))'),                       # PLAIN x(P) for pt-add\n", '')
s = s.replace("    ('pyP', '(fm pky (c-onep 0))'),                       # PLAIN even-y(P)\n", '')
s = s.replace("    ('PJ', '(list pxP pyP (c-onem 0))'),", "    ('PJ', '(list pxm pky (c-onem 0))'),                  # rep coords, Z=rep(1)")
s = s.replace("    ('gxP', '(fm (c-gxm 0) (c-onep 0))'),                 # PLAIN Gx\n", '')
s = s.replace("    ('gyP', '(fm (c-gym 0) (c-onep 0))'),                 # PLAIN Gy\n", '')
s = s.replace("    ('GJ', '(list gxP gyP (c-onem 0))'),", "    ('GJ', '(list (c-gxm 0) (c-gym 0) (c-onem 0))'),      # rep coords, Z=rep(1)")

# 5) ladder equality operands: lhs = X(R') raw rep, rhs = rm*Z^2 rep
s = s.replace("    ('lhs', '(vec-nth Pf 0)'),                            # X(R') raw (carries Z^2 factor)",
              "    ('lhs', '(vec-nth Pf 0)'),                            # X(R') raw rep")
s = s.replace("    ('rhs', '(fm rm z2)'),                                # r * Z^2  -- same Z^2 factor both sides",
              "    ('rhs', '(fm rm z2)'),                                # r * Z^2 rep -- scale-matched")

open(P, 'w').write(s)
ast.parse(s)

# report leftovers
for bad in ('fe-eqn', 'pxP', 'pyP', 'gxP', 'gyP', '(fm nyp (c-onep 0)) 7'):
    print(f"residual {bad!r}:", s.count(bad))
print("fe-eqc count:", s.count('fe-eqc'))
print("ast OK")
