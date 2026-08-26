#!/usr/bin/env python3
"""Build examples/bip340.lisp from parts:
   1. core from scripts/gen_bip340.py (already run) -> /tmp/bip340_core.lisp
   2. fix sha-blk grouped-let paren bug (6x)
   3. fix blk0..4 returns to raw (keep Davies-Meyer add only in blk5)
   4. append sha256-words driver (BE words, arithmetic xor)
   5. convert old-style points.lisp fe calls to new 10-arg style
   6. + /tmp/verify_fn.lisp + /tmp/bip_tests.lisp (or smoke tests)
Usage: build_bip.py [--smoke]
"""
import re, sys

SMOKE = '--smoke' in sys.argv

# ── 1+2+3: core fixes ──
core = open('/tmp/bip340_core.lisp').read()

BAD_LET = ("(let (((a h0) (b h1) (c h2) (d h3) (e h4) (f h5) (g h6) (h h7)) "
           "(t1 0) (t2 0) (Wx (list)))")
GOOD_LET = ("(let ((a h0) (b h1) (c h2) (d h3) (e h4) (f h5) (g h6) (h h7) "
            "(t1 0) (t2 0) (Wx (list)))")
n_let = core.count(BAD_LET)
assert n_let == 6, "expected 6 bad lets, got %d" % n_let
core = core.replace(BAD_LET, GOOD_LET)

BAD_RET = ("(list (band (+ h0 a) 4294967295) (band (+ h1 b) 4294967295) "
           "(band (+ h2 c) 4294967295) (band (+ h3 d) 4294967295) "
           "(band (+ h4 e) 4294967295) (band (+ h5 f) 4294967295) "
           "(band (+ h6 g) 4294967295) (band (+ h7 h) 4294967295))")
RAW_RET = "(list a b c d e f g h)"
n_ret = core.count(BAD_RET)
assert n_ret == 6, "expected 6 add-returns, got %d" % n_ret
# replace ALL 6 (blk5's add moves to the driver — its h-args are the
# chunk-4 WORKING state, not the block's original state)
idx = -1
for _ in range(6):
    idx = core.index(BAD_RET, idx + 1)
    core = core[:idx] + RAW_RET + core[idx + len(BAD_RET):]
assert core.count(BAD_RET) == 0

# ── fix round-state cascade order: emitted top-down (each set! reads the
#    ALREADY-updated var). Correct: h←g, g←f, f←e, e←d+t1, d←c, c←b, b←a, a←t1+t2
BAD_CASC = ("    (set! g h)\n"
            "    (set! f g)\n"
            "    (set! e (band (+ d t1) 4294967295))\n"
            "    (set! d e)\n"
            "    (set! c d)\n"
            "    (set! b c)\n"
            "    (set! a (band (+ a t2) 4294967295))\n")
GOOD_CASC = ("    (set! h g)\n"
             "    (set! g f)\n"
             "    (set! f e)\n"
             "    (set! e (band (+ d t1) 4294967295))\n"
             "    (set! d c)\n"
             "    (set! c b)\n"
             "    (set! b a)\n"
             "    (set! a (band (+ t1 t2) 4294967295))\n")
n_casc = core.count(BAD_CASC)
assert n_casc >= 64 and n_casc % 64 == 0, "unexpected cascade count %d" % n_casc
core = core.replace(BAD_CASC, GOOD_CASC)

# ── fix Wx literal: generator truncated it to 1 entry; rounds read Wx[0..63].
#    blk_i uses schedule window W[16i .. 16i+63]
import re as _re2

def fix_all_wx(core):
    # find each sha-blkN function span
    spans = []
    for m in _re2.finditer(r"\(define \(sha-blk(\d)", core):
        blk_no = int(m.group(1))
        nxt = _re2.search(r"\n\(define ", core[m.end():])
        end = m.end() + nxt.start() if nxt else len(core)
        spans.append((m.start(), end, blk_no))
    out, pos = [], 0
    for a, b, blk_no in spans:
        seg = core[a:b]
        off = 16 * blk_no
        entries = " ".join("(vec-nth W %d)" % (off + k) for k in range(64))
        seg2, n = _re2.subn(r"\(set! Wx \(list .*\)\)",
                            "(set! Wx (list %s))" % entries, seg)
        assert n == 1, "blk%d: %d Wx sets" % (blk_no, n)
        out.append(core[pos:a])
        out.append(seg2)
        pos = b
    out.append(core[pos:])
    return "".join(out)

n_wx = len(_re2.findall(r"\(define \(sha-blk\d", core))
assert n_wx == 6, "expected 6 blks, got %d" % n_wx
core = fix_all_wx(core)

# ── 4: sha driver ──
sha = []
def A(s=''):
    sha.append(s)
A("(define (wxor a b)")
A("  (band (- (+ a b) (+ (band a b) (band a b))) 4294967295))")
A()
A("(define (w-sig0 x)")
A("  (let ((r7 (band (bor (shr x 7) (shl x 25)) 4294967295)))")
A("    (let ((r18 (band (bor (shr x 18) (shl x 14)) 4294967295)))")
A("      (wxor (wxor r7 r18) (shr x 3)))))")
A()
A("(define (w-sig1 x)")
A("  (let ((r17 (band (bor (shr x 17) (shl x 15)) 4294967295)))")
A("    (let ((r19 (band (bor (shr x 19) (shl x 13)) 4294967295)))")
A("      (wxor (wxor r17 r19) (shr x 10)))))")
A()
A("(define (w-sched B)")
A("  (loop ((Wi B) (i 16))")
A("    (if (= i 64) Wi")
A("      (let ((x15 (vec-nth Wi (- i 15))))")
A("        (let ((x2 (vec-nth Wi (- i 2))))")
A("          (let ((w16 (vec-nth Wi (- i 16))))")
A("            (let ((w7 (vec-nth Wi (- i 7))))")
A("              (recur (vec-push Wi (band (+ (+ (+ w16 (w-sig0 x15)) w7) (w-sig1 x2)) 4294967295)) (+ i 1))))))))")
A()
A("(define (sha256-words w nw tail tlen)")
A("  (let ((padw (if (= tlen 0) 2147483648 (bor tail (shl 128 (* 8 (- 3 tlen)))))))")
A("    (let ((bitlen (+ (* 32 nw) (* 8 tlen))))")
A("      (let ((W1 (vec-push w padw)))")
A("        (let ((Wp (loop ((Wv W1))")
A("                    (if (= (mod (vec-length Wv) 16) 14) Wv")
A("                        (recur (vec-push Wv 0))))))")
A("          (let ((Wf (vec-push (vec-push Wp 0) bitlen)))")
A("            (let ((S0 (list 1779033703 3144134277 1013904242 2773480762 1359893119 2600822924 528734635 1541459225)))")
A("              (let ((nb (/ (vec-length Wf) 16)))")
A("                (loop ((Si S0) (bi 0))")
A("                  (if (= bi nb) Si")
A("                    (let ((B (loop ((Bi (list)) (k 0))")
A("                                (if (= k 16) Bi")
A("                                  (recur (vec-push Bi (vec-nth Wf (+ (* bi 16) k))) (+ k 1))))))")
A("                      (let ((W (w-sched B)))")
A("                        (let ((C (sha-blk0 W %s)))"
  % " ".join("(vec-nth Si %d)" % t for t in range(8)))
A("                          (recur (list %s) (+ bi 1))))))))))))))"
  % " ".join("(band (+ (vec-nth Si %d) (vec-nth C %d)) 4294967295)" % (t, t) for t in range(8)))
sha_text = "\n".join(sha) + "\n"

# per-define balance repair: close each define-block individually
import re as _re
blocks = sha_text.split('(define ')
fixed = []
for k, blk in enumerate(blocks):
    if k == 0:
        fixed.append(blk); continue
    net = blk.count('(') - blk.count(')') + 1  # +1: split removed the define's own '('
    assert net >= 0, "sha block %d over-closed" % k
    fixed.append(blk + ')' * net)
sha_text = '(define '.join(fixed)
assert sha_text.count("(") == sha_text.count(")")

# ── 5: points conversion ──
pts_src = open('/tmp/bip_parts/points.lisp').read()
pts_src = pts_src[pts_src.index('(define (pt-inf'):]
VEC = re.compile(r'^\(vec-nth ([A-Za-z0-9_+-]+) (\d)\)$')

def split_top(s):
    args, depth, cur = [], 0, ''
    for ch in s:
        if ch == '(':
            depth += 1; cur += ch
        elif ch == ')':
            depth -= 1; cur += ch
        elif ch == ' ' and depth == 0:
            args.append(cur); cur = ''
        else:
            cur += ch
    if cur:
        args.append(cur)
    return args

def convert(text):
    out, i, nconv = [], 0, 0
    while True:
        j = text.find('(fe-', i)
        if j == -1:
            out.append(text[i:]); break
        k = j + 1
        while text[k] != ' ':
            k += 1
        op = text[j+1:k]
        depth, m = 0, j
        while True:
            if text[m] == '(':
                depth += 1
            elif text[m] == ')':
                depth -= 1
                if depth == 0:
                    break
            m += 1
        call = text[j:m+1]
        out.append(text[i:j])
        if op in ('fe-mul', 'fe-add', 'fe-sub', 'fe-eq'):
            args = split_top(call[len(op)+2:-1])
            if len(args) == 18:
                ms = [VEC.match(a.strip()) for a in args[9:]]
                if all(ms) and all(ms[t].group(2) == str(t) for t in range(9)) \
                   and len({m_.group(1) for m_ in ms}) == 1:
                    call = '(%s %s %s)' % (op, ' '.join(a.strip() for a in args[:9]), ms[0].group(1))
                    nconv += 1
        out.append(call)
        i = m + 1
    return ''.join(out), nconv

pts, nconv = convert(pts_src)
assert nconv >= 40, "only %d fe calls converted" % nconv
assert pts.count("(") == pts.count(")"), "points unbalanced after conversion"

# final guard: no nested defines, depth returns to 0
def audit(text):
    lines = [l.split(';')[0] for l in text.split('\n')]
    depth = 0
    for i, ln in enumerate(lines, 1):
        for j, ch in enumerate(ln):
            if ch == '(' and ln[j:j+7] == 'define' and depth > 0:
                raise AssertionError("nested define at line %d" % i)
            if ch == '(':
                depth += 1
            elif ch == ')':
                depth -= 1
        assert depth >= 0, "depth negative at line %d" % i
    assert depth == 0, "final depth %d" % depth

# ── 6: assemble ──
# inject missing c-* constants used by verify (one-liners)
LIMB, NL = 30, 9
MASK = (1 << LIMB) - 1
_p = 2**256 - 2**32 - 977
def _limbs(x, nl=NL):
    return [(x >> (LIMB * i)) & MASK for i in range(nl)]
CONSTS = {
    "c-one": 1,
    "c-zero": 0,
    "c-pm": _p - 1,
}
inject = []
for name, val in CONSTS.items():
    if "(define (%s " % name not in core:
        inject.append("(define (%s _d) (list %s))" % (name, " ".join(str(v) for v in _limbs(val))))
        print("injected constant:", name)
if inject:
    core = core + "\n" + "\n".join(inject) + "\n"

parts = [core, sha_text, pts]
if SMOKE:
    parts.append(open('/tmp/bip_smoke.lisp').read())
else:
    parts.append(open('/tmp/verify_fn.lisp').read())
    parts.append(open('/tmp/bip_tests.lisp').read())
final = "\n".join(parts)
lines = [l.split(';')[0] for l in final.split('\n')]
net = sum(l.count('(') - l.count(')') for l in lines)
assert net == 0, "final net %d" % net
audit(final)
out = 'examples/bip340_smoke.lisp' if SMOKE else 'examples/bip340.lisp'
open(out, 'w').write(final)
print("built %s: %d lines, %d fe-conv, balance ok" % (out, len(lines), nconv))
