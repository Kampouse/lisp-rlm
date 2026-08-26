#!/usr/bin/env python3
"""Rebuild verify_fn.lisp with a structural builder (no hand-counted parens)."""
import csv, hashlib

NL = 9
p = 2**256 - 2**32 - 977

def limbs(x, nl=NL):
    return [(x >> (30 * i)) & ((1 << 30) - 1) for i in range(nl)]

def lo(a):
    return " ".join("(vec-nth %s %d)" % (a, i) for i in range(NL))

def fe2(fn, a, b):
    return "(%s %s %s)" % (fn, lo(a), b)

def balanced(s):
    d = 0
    for ch in s:
        if ch == '(':
            d += 1
        elif ch == ')':
            d -= 1
            assert d >= 0, "over-close: ..." + s[:80]
    return d == 0

# ── innermost-out construction ──
# scalar-mul ladder: r·z2 where z2 = Z² (Z = Pf's Z coord)
loop_txt = (
    "(loop ((Pt P0) (i 255))"
    "  (let ((Pt2 (pt-dbl (vec-nth Pt 0) (vec-nth Pt 1) (vec-nth Pt 2))))"
    "    (let ((sbit (band (shr (vec-nth s (/ i 30)) (mod i 30)) 1)))"
    "      (let ((kbit (band (shr (vec-nth k (/ i 30)) (mod i 30)) 1)))"
    "        (let ((Pt3 (if (= sbit 1)"
    "                       (pt-add (vec-nth Pt2 0) (vec-nth Pt2 1) (vec-nth Pt2 2)"
    "                               (vec-nth GJ 0) (vec-nth GJ 1) (vec-nth GJ 2))"
    "                       Pt2)))"
    "          (let ((Pt4 (if (= kbit 1)"
    "                         (pt-add (vec-nth Pt3 0) (vec-nth Pt3 1) (vec-nth Pt3 2)"
    "                                 (vec-nth PJ 0) (vec-nth PJ 1) (vec-nth PJ 2))"
    "                         Pt3)))"
    "            (if (= i 0) Pt4 (recur Pt4 (- i 1))))))))"
)
_d = loop_txt.count("(") - loop_txt.count(")")
assert _d >= 0, "loop over-closed"
loop_txt = loop_txt + ")" * _d
assert balanced(loop_txt), "loop unbalanced"

LADDER = [
    ("pxm", "(fe-mul %s (c-r2 0))" % lo("pk")),
    ("sq", fe2("fe-mul", "pxm", "pxm")),
    ("cube", fe2("fe-mul", "sq", "pxm")),
    ("c1", fe2("fe-add", "cube", "(c-sevenm 0)")),
    ("y", "(fe-sqrt %s)" % lo("c1")),
    ("ysq", fe2("fe-mul", "y", "y")),
    ("yn", "(fe-mul %s (c-one 0))" % lo("y")),
    ("y2", "(if (= (band (vec-nth yn 0) 1) 1) (fe-sub %s yn) yn)" % lo("(c-p 0)")),
    ("rm", "(fe-mul %s (c-r2 0))" % lo("r")),
    ("rsq", fe2("fe-mul", "rm", "rm")),
    ("rcube", fe2("fe-mul", "rsq", "rm")),
    ("cr", fe2("fe-add", "rcube", "(c-sevenm 0)")),
    ("yr", "(fe-sqrt %s)" % lo("cr")),
    ("yrsq", fe2("fe-mul", "yr", "yr")),
    ("tagw", "(c-tagw 0)"),
    ("rwb", "(fe-words-be r)"),
    ("pwb", "(fe-words-be pk)"),
    ("pre0", "(list %s %s %s)" % (
        " ".join("(vec-nth tagw %d)" % i for i in range(8)),
        " ".join("(vec-nth rwb %d)" % i for i in range(8)),
        " ".join("(vec-nth pwb %d)" % i for i in range(8)))),
    ("pre", "(loop ((i 0) (acc pre0)) (if (= i nw) acc"
            " (recur (+ i 1) (vec-push acc (vec-nth msg i)))))"),
    ("dg", "(sha256-words pre (+ nw 24) tail tlen)"),
    ("el", "(words-limbs %s)" % " ".join("(vec-nth dg %d)" % i for i in range(8))),
    ("er", "(sc-reduce %s)" % lo("el")),
    ("k", "(sc-n-minus %s)" % lo("er")),
    ("gx", "(c-gx 0)"),
    ("gy", "(c-gy 0)"),
    ("onem", "(c-onem 0)"),
    ("GJ", "(list gx gy onem)"),
    ("PJ", "(list pxm y2 onem)"),
    ("P0", "(pt-inf 0)"),
    ("Pf", loop_txt),
    ("z2", "(fe-mul %s (vec-nth Pf 2))" % lo("(vec-nth Pf 2)")),
    ("rhs", fe2("fe-mul", "rm", "z2")),
]
for name, val in LADDER:
    assert balanced(val), "val %s unbalanced: %s" % (name, val[:80])

RESULT = "(fe-eq %s rhs)" % lo("(vec-nth Pf 0)")
assert balanced(RESULT)

# guards fold inside-out: innermost body = ladder(final let) ... build:
body = RESULT
for name, val in reversed(LADDER):
    body = "(let ((%s %s))\n  %s)" % (name, val, body)
for guard in ["(= (sc-geq-n %s) 1)" % lo("s"),
              "(= (sc-geq-p %s) 1)" % lo("r"),
              "(= (sc-geq-p %s) 1)" % lo("pk")]:
    body = "(if %s 0\n  %s)" % (guard, body)

fn = "(define (bip340-verify pk r s msg nw tail tlen)\n  %s)\n" % body
assert balanced(fn), "verify fn unbalanced"
assert fn.count("(") == fn.count(")")

open("/tmp/verify_fn.lisp", "w").write(fn)

# ── tests (single combined test — per-test typecheck is ~7min) ──
w0 = int.from_bytes(hashlib.sha256(b"").digest()[0:4], "big")
mont = ("(let ((a (fe-mul 2 0 0 0 0 0 0 0 0 (c-r2 0))))"
        "(let ((b (fe-mul 3 0 0 0 0 0 0 0 0 (c-r2 0))))"
        "(let ((c (fe-mul %s b)))" % " ".join("(vec-nth a %d)" % i for i in range(9)) +
        "(let ((d (fe-mul 6 0 0 0 0 0 0 0 0 (c-r2 0))))"
        "(fe-eq %s d)))))" % " ".join("(vec-nth c %d)" % i for i in range(9)))
shae = "(vec-nth (sha256-words (list) 0 0 0) 0)"
shabc = "(vec-nth (sha256-words (list) 0 %d 3) 0)" % int.from_bytes(b"abc\x00", "big")

def words_of(msg_bytes):
    full = msg_bytes + b"\x00" * ((-len(msg_bytes)) % 4)
    words = [int.from_bytes(full[i:i+4], "big") for i in range(0, len(full), 4)]
    if len(msg_bytes) % 4 == 0:
        return words, 0, 0
    tlen = len(msg_bytes) % 4
    tail = words.pop()
    return words, tail, tlen

rows = list(csv.DictReader(open("/tmp/bip340_vectors.csv")))
exprs = [mont, shae, shabc]
expected = [1, w0, int.from_bytes(hashlib.sha256(b"abc").digest()[0:4], "big")]
for row in rows:
    pk = "(list %s)" % " ".join(str(v) for v in limbs(int(row["public key"], 16)))
    sig = row["signature"]
    r_ = "(list %s)" % " ".join(str(v) for v in limbs(int(sig[:64], 16)))
    s_ = "(list %s)" % " ".join(str(v) for v in limbs(int(sig[64:128], 16)))
    mb = bytes.fromhex(row["message"])
    wds, tail, tlen = words_of(mb)
    wl = "(list %s)" % " ".join(str(w) for w in wds) if wds else "(list)"
    exp = 1 if row["verification result"].strip().upper() == "TRUE" else 0
    exprs.append("(bip340-verify %s %s %s %s %d %d %d)" % (pk, r_, s_, wl, len(wds), tail, tlen))
    expected.append(exp)

single = ['(test "bip340-all"',
          "  (list"] + \
         ["    %s" % e for e in exprs[:-1]] + \
         ["    %s)" % exprs[-1],
          "  (list %s))" % " ".join(str(e) for e in expected)]
open("/tmp/bip_tests.lisp", "w").write("\n".join(single) + "\n")

smoke = ['(test "smoke"',
         "  (list %s %s)" % (exprs[0], exprs[1]),
         "  (list 1 %d))" % w0]
open("/tmp/bip_smoke.lisp", "w").write("\n".join(smoke) + "\n")
print("verify built structurally; %d exprs total" % len(exprs))
