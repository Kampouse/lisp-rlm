#!/usr/bin/env python3
"""v0 oracle + lisp probe for the tagged-hash digest layer."""
import hashlib

pk = bytes.fromhex("D69C3509BB99E412E68B0FE85054E77674D7F4BF9B93DBF7EBF5F1BFDE2F27C3")
r_ = bytes.fromhex("00000000000000000000000493E9C1F4B5D0AF2E96D5A227A336FCD9F43490FD")
msg = bytes.fromhex("0000000000000000000000000000000000000000000000000000000000000000")

def tagged(tag, data):
    t = hashlib.sha256(tag.encode()).digest()
    return hashlib.sha256(t + t + data).digest()

e = tagged("BIP0340/challenge", r_ + pk + msg)
print("e digest BE words:", [int.from_bytes(e[i:i+4], "big") for i in range(0, 32, 4)])

NL = 9
def limbs(x):
    return [(x >> (30 * i)) & ((1 << 30) - 1) for i in range(NL)]

def lo(a):
    return " ".join("(vec-nth %s %d)" % (a, i) for i in range(NL))

def words_of(msg_bytes):
    full = msg_bytes + b"\x00" * ((-len(msg_bytes)) % 4)
    words = [int.from_bytes(full[i:i+4], "big") for i in range(0, len(full), 4)]
    if len(msg_bytes) % 4 == 0:
        return words, 0, 0
    tlen = len(msg_bytes) % 4
    tail = words.pop()
    return words, tail, tlen

mw, tail, tlen = words_of(msg)
tagw_words = [int.from_bytes(hashlib.sha256(b"BIP0340/challenge").digest()[i:i+4], "big") for i in range(0, 32, 4)]

# lisp probe: replicate verify's pre construction, hash it, compare word 0
src = open("examples/bip340.lisp").read()
probe = []
probe.append("(define (probe-e pk r msg nw tail tlen)")
probe.append("  (let ((pxm (fe-mul %s (c-r2 0))))" % lo("pk"))
probe.append("    (let ((rm (fe-mul %s (c-r2 0))))" % lo("r"))
probe.append("      (let ((tagw (c-tagw 0)))")
probe.append("        (let ((rwb (fe-words-be rm)))")
probe.append("          (let ((pwb (fe-words-be pxm)))")
tag8 = " ".join("(vec-nth tagw %d)" % i for i in range(8))
probe.append("            (let ((pre0 (list %s %s %s %s)))" % (tag8, tag8,
             " ".join("(vec-nth rwb %d)" % i for i in range(8)),
             " ".join("(vec-nth pwb %d)" % i for i in range(8))))
probe.append("              (let ((pre (loop ((i 0) (acc pre0)) (if (= i nw) acc (recur (+ i 1) (vec-push acc (vec-nth msg i)))))))")
probe.append("                (sha256-words pre (+ nw 32) tail tlen)))))")
probe.append(")")
pkl = "(list %s)" % " ".join(str(v) for v in limbs(int.from_hex if False else int.fromhex(pk.hex(), 16)))
rl = "(list %s)" % " ".join(str(v) for v in limbs(int(r_.hex(), 16)))
wl = "(list %s)" % " ".join(str(w) for w in mw)
dwords = [int.from_bytes(e[i:i+4], "big") for i in range(0, 32, 4)]
tests = ['(test "e%d" (vec-nth (probe-e %s %s %s %d %d %d) %d) %d)'
         % (k, pkl, rl, wl, len(mw), tail, tlen, k, dwords[k]) for k in range(8)]
open("/tmp/tp/probe_e.lisp", "w").write(src[:src.index('(define (bip340-verify')] + "\n".join(probe) + "\n" + "\n".join(tests) + "\n")
print("probe_e written; expected w0 =", dwords[0])
