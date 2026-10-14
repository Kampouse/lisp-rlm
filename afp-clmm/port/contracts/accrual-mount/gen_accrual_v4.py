#!/usr/bin/env python3
"""accrual-mount v4 assembly: build the mounted pool from Jean's lib fragments.

Reads lib/{clmm_core,clmm_pool,clmm_pool_v4}.lisp VERBATIM, applies
anchor-checked surgical transforms (each anchor must match EXACTLY ONCE —
drift between lib/ and this generator is a hard error), and emits:

  contracts/accrual-mount/v4-pool/src/main.built.lisp   (buildable pool)
  contracts/accrual-mount/v4-pool/near.json
  scenarios/lisp/accrual-mount-v4/s.json                (near-mock scenario)

The scenario's expected values come from a PYTHON MIRROR of the pool's
semantics (ladder walk, fee bps, share mint/burn, PS/CK/BAL accrual)
implemented below — same equations as the mounted contract, so every
assert is computed, never hand-transcribed. lib/ files are never edited.

Mounted hooks (accrual-mount.md H1-H4):
  H1 swap   : G += f (f = fee-get delta, non-refund branch only);
              reversal args carry "p" = the exact increment (pay_out
              restores G bit-exactly — a reverted swap never mints claims)
  H2 deposit: settle then CK:<lp> := G (after PB store, before SH mint —
              settle reads PRE-mint shares)
  H3 claim  : owed = (G-CK)*sh div 2^64 — the twin's accruef with Lf = sh
              (G increments are per-share at wave time: f*2^64//SHT);
  H4 withdraw: settle BEFORE burn; owed rides the A payout leg
              (tot_a = out_a + owed); pay_b fail-leg restores CK/BAL to
              the PRE-SETTLE values carried in args ("kc"/"bc") — deleting
              the checkpoint would reset ck to 0 and OVERPAY next claim,
              so exact restore, never delete.

Also exports init/withdraw (defined-but-unexported in v4 — finding).
"""
import json, os, re

HERE = os.path.dirname(os.path.abspath(__file__))
PORT = os.path.abspath(os.path.join(HERE, "..", ".."))
REPO = os.path.abspath(os.path.join(PORT, "..", ".."))
LIB = os.path.join(REPO, "lib")
OUT_DIR = os.path.join(HERE, "v4-pool", "src")
OUT = os.path.join(OUT_DIR, "main.built.lisp")
V4 = os.path.join(HERE, "v4-pool")
SCN_DIR = os.path.join(PORT, "scenarios", "lisp", "accrual-mount-v4")
SCN = os.path.join(SCN_DIR, "s.json")
TWO64 = 1 << 64

# ---------------------------------------------------------------- mirror
class Pool:
    """Python mirror of the mounted pool semantics (decimal-string math).

    Literal port of lib/clmm_pool.lisp (clmm-step4/pool-swap), the v4
    slot-widen/add-b-liq/withdraw4, and the accrual defs. Every formula
    below is transcribed from the emitted lisp, not re-derived.
    """
    SHAPE = ["1000", "2000", "5000", "3000", "1500"]
    PNUM = ["98", "99", "100", "101", "102"]

    def __init__(self):
        self.s = ["0"] * 5          # S0..S4 remaining A liquidity per slot
        self.F = "0"; self.AB = "0"; self.PB = "0"
        self.SHT = "0"; self.G = "0"
        self.SH = {}; self.CK = {}; self.BAL = {}
        self.DY = "0"; self.REM = "0"
        self.floatA = "0"; self.floatB = "0"
        self.tk = {"A": {}, "B": {}}

    # -- bignum helpers (decimal strings) --
    def add(self, x, y): return str(int(x) + int(y))
    def sub(self, x, y): return str(int(x) - int(y))
    def muldiv(self, x, y, z): return str(int(x) * int(y) // int(z))

    # -- accrual (acm-get/acm-settle/acm-grow) --
    def settle(self, lp):
        sh = self.SH.get(lp, "0")
        ck = self.CK.get(lp, "0")
        g = self.G
        if int(g) < int(ck): return "0"
        if self.SHT == "0":
            self.CK[lp] = g
            return "0"
        owed = self.muldiv(self.sub(g, ck), sh, str(2**64))
        self.BAL[lp] = self.add(self.BAL.get(lp, "0"), owed)
        self.CK[lp] = g
        return owed

    def grow(self, f):
        # M1a: pure compute -- G commits in swap_a's success fork (pay_out)
        return self.muldiv(f, str(2**64), self.SHT) if self.SHT != "0" else "0"

    # -- clmm-step4 / pool-swap (per-slot walk, fee floor PER SLOT) --
    def pool_swap(self, amt):
        dx, dy, fe = int(amt), 0, 0
        for i in range(5):
            la = int(self.s[i])
            if dx <= la:
                used, left = dx, la - dx
            else:
                used, left = la, 0
            dy += used * int(self.PNUM[i]) // 100
            fe += used * 30 // 10000
            dx -= used
            self.s[i] = str(left)
        self.F = self.add(self.F, str(fe))
        self.DY = str(dy); self.REM = str(dx)

    # -- slot-widen / add-b-liq (H2) --
    def slot_widen(self, i, base, amt, tot):
        self.s[i] = self.add(self.s[i],
            self.muldiv(self.muldiv(amt, base, tot), "100", self.PNUM[i]))

    def add_b_liq(self, lp, amt):
        if int(amt) < 10:
            return amt                              # dust refund
        sh = amt if self.SHT == "0" else self.muldiv(amt, self.SHT, self.PB)
        if sh == "0":
            return amt                              # zero-share refund
        tot = str(sum(int(x) for x in self.s))
        if tot == "0":
            for i in range(5):                       # shape bootstrap
                self.slot_widen(i, self.SHAPE[i], amt, "12500")
        else:
            for i in range(5):                       # proportional widen
                self.slot_widen(i, self.s[i], amt, tot)
        self.SHT = self.add(self.SHT, sh)
        self.PB = self.add(self.PB, amt)
        self.settle(lp)                             # H2 settle-then-ckpt
        self.CK[lp] = self.G
        self.SH[lp] = self.add(self.SH.get(lp, "0"), sh)
        # direct ft_on_transfer deposit: no real token leg moves
        return "0"

    # -- v4 swap wrapper (H1) --
    def swap_a(self, sender, amt, min_dy, fail_b=False):
        f0 = self.F
        g0 = self.G
        s0 = list(self.s)
        self.pool_swap(amt)
        dy, rem = self.DY, self.REM
        if int(dy) < int(min_dy):
            self.s = s0; self.F = f0                # restore, full refund
            return {"refund": amt}
        f1 = self.sub(self.F, f0)
        ab0, pb0 = self.AB, self.PB
        inc = self.grow(f1)                         # H1 (pure compute)
        if fail_b:
            # pool B token float empty -> payout promise traps -> pay-out
            # fail leg (M1a): F := f0; slots back (pool_swap wrote them in
            # the mirror model); G/AB/PB were NEVER touched (commit is in
            # pay_out success) -- reversal is the identity on them.
            self.s = s0; self.F = f0
            assert self.G == g0, "H1 reversal: G moved on failed swap"
            assert self.AB == ab0 and self.PB == pb0, "H1: AB/PB moved on fail"
            return {"refund": amt, "dy_trapped": dy, "inc_reversed": inc}
        # success: pay_out commits the whole wave (M1a)
        self.G = self.add(g0, inc)
        self.AB = self.add(ab0, self.sub(amt, rem))
        self.PB = self.sub(pb0, dy)
        # transport truth: swap A arrives via direct ft_on_transfer (no real
        # token leg); the B payout IS a real promise leg.
        self.floatB = self.sub(self.floatB, dy)
        self.tk["B"][sender] = self.add(self.tk["B"].get(sender, "0"), dy)
        if rem != "0":                              # unused-A refund
            self.tk["A"][sender] = self.add(self.tk["A"].get(sender, "0"), rem)
        return {"dy": dy, "rem": rem, "f": f1, "p": inc}

    def claim(self, lp):
        return self.settle(lp)

    # -- withdraw4 (H3 settle pre-burn, H4 owed rides A leg) --
    def withdraw(self, lp, sh, fail_a=False):
        assert int(self.SH.get(lp, "0")) >= int(sh), "insufficient shares"
        kc = self.CK.get(lp, "0")
        bc = self.BAL.get(lp, "0")
        owed = self.settle(lp)                      # H3 pre-burn
        ab, pb, sht = self.AB, self.PB, self.SHT
        o0_list = list(self.s)
        out_a = self.muldiv(sh, ab, sht)
        out_b = self.muldiv(sh, pb, sht)
        tot_a = self.add(out_a, self.add(bc, owed))  # H4: out_a + bc + owed
        self.SHT = self.sub(sht, sh)
        left = self.sub(self.SH[lp], sh)
        if left == "0":
            self.SH.pop(lp, None)
        else:
            self.SH[lp] = left
        self.AB = self.sub(ab, out_a)
        self.PB = self.sub(pb, out_b)
        for i in range(5):
            self.s[i] = self.muldiv(self.s[i], self.PB, pb)
        self.floatA = self.sub(self.floatA, tot_a)
        self.floatB = self.sub(self.floatB, out_b)
        self.tk["A"][lp] = self.add(self.tk["A"].get(lp, "0"), tot_a)
        self.tk["B"][lp] = self.add(self.tk["B"].get(lp, "0"), out_b)
        if fail_a:
            # pool A token float empty -> A payout promise traps -> pay_b
            # fail leg: SHT/SH/AB/PB/slots restored + ck:=kc, bal:=bc.
            self.SHT = sht          # direct restore (was already reduced)
            if left == "0":
                self.SH[lp] = sh
            else:
                self.SH[lp] = self.add(left, sh)
            self.AB = ab            # fail leg adds back exactly out_a
            self.PB = pb            # fail leg adds back exactly out_b
            # slots were scaled by PB/pb; pay_b adds back x_i = o_i - scaled,
            # net effect slots return to o_i exactly (string adds, no division).
            self.s = list(o0_list)
            self.CK[lp] = kc
            self.BAL[lp] = bc
            # trapped transfer never moved tokens: undo float/tk deltas
            self.floatA = self.add(self.floatA, tot_a)
            self.floatB = self.add(self.floatB, out_b)
            self.tk["A"][lp] = self.sub(self.tk["A"][lp], tot_a)
            self.tk["B"][lp] = self.sub(self.tk["B"][lp], out_b)
            assert self.SH[lp] == sh and self.SHT == sht
            return {"status": "wd-failed", "owed": owed, "tot_a": tot_a,
                    "kc": kc, "bc": bc}
        self.BAL[lp] = "0"                          # realized credit paid out
        return {"out_a": out_a, "owed": owed, "tot_a": tot_a,
                "out_b": out_b, "kc": kc, "bc": bc}

# ------------------------------------------------------------- transform
ACCRUAL_DEFS = '''
;; ---- accrual mount (generated; accrual-mount.md H1-H4) ----
;; G: aggregate fee growth, 2^64 fixed point. CK:<lp>: per-LP checkpoint.
;; owed = (G-CK)*sh div 2^64 -- twin accruef, Lf = sh (constant cap).
;; G increments are ALREADY per-share (f*2^64//SHT at wave time), so the
;; claim divides by 2^64 exactly once; owed is RAW A units, consistent
;; with out_a in the withdraw payout leg.
(define (acm-ck-key a) (str-cat "CK:" a))
(define (acm-bal-key a) (str-cat "BAL:" a))
(define (acm-get k) (let ((v (near/load-bytes k))) (if (nil? v) "0" v)))

;; H3: settle LP -- owed on PRE-call shares, credit BAL, ck := G
;; SHT=0 guard: first deposit has no share base yet (owed 0, ck := G).
(define (acm-settle lp)
  (let ((sh (acm-get (sh-key lp))))
    (let ((ck (acm-get (acm-ck-key lp))))
      (let ((g (acm-get "G")))
        (if (< (li-cmp g ck) 0)
            "0"
            (if (str= (acm-get "SHT") "0")
                (begin
                  (near/store-bytes (acm-ck-key lp) g)
                  "0")
                (let ((owed (u128-muldiv (li-sub g ck) sh TWO64)))
                  (begin
                    (near/store-bytes (acm-bal-key lp)
                      (li-add (acm-get (acm-bal-key lp)) owed))
                    (near/store-bytes (acm-ck-key lp) g)
                    owed))))))))

;; H1: grow the aggregate by a fee wave (non-refund branch only)
(define (acm-grow f)
  ;; M1a: PURE compute -- returns the increment without storing. G's commit
  ;; moved into pay_out success ("g"+"p" args). store-bytes itself returns
  ;; "" -- a value that rides args MUST be a real string (H1 exactness).
  (if (str= (bz "SHT") "0")
      "0"
      (u128-muldiv f TWO64 (bz "SHT"))))

;; explicit claim: settle into BAL (payout rides the withdraw A leg)
(define (acm-claim)
  (near/return (acm-settle (near/predecessor_account_id))))

;; view: full accrual state for scenario pins
(define (accrual-state)
  (let ((lp (json-get-str "lp" (near/input))))
    (str-cat "G:" (acm-get "G")
             " F:" (fee-get)
             " SHT:" (acm-get "SHT")
             " AB:" (acm-get "AB")
             " PB:" (acm-get "PB")
             " SH:" (acm-get (sh-key lp))
             " CK:" (acm-get (acm-ck-key lp))
             " BAL:" (acm-get (acm-bal-key lp)))))
'''

PAY_SECTION = '''(define (pay-out)
  ;; M1a: the swap wave commits HERE, not in ft-on-transfer4. Success:
  ;; F/G/AB/PB from pre-wave absolutes + wave deltas; slots from the tuple.
  ;; Fail: restore the four pre-wave absolutes -- slots were never touched.
  (if (self-call?)
      (let ((inp (near/input)))
        (let ((res (near/promise_result 0)))
          (if (< (str-length res) 1)
              (begin
                (near/store-bytes "F" (json-get-str "f" inp))
                (near/store-bytes "G" (json-get-str "g" inp))
                (near/store-bytes "AB" (json-get-str "na" inp))
                (near/store-bytes "PB" (json-get-str "np" inp))
                (near/return (json-get-str "a" inp)))
              (begin
                (slot-set 0 (json-get-str "c0" inp))
                (slot-set 1 (json-get-str "c1" inp))
                (slot-set 2 (json-get-str "c2" inp))
                (slot-set 3 (json-get-str "c3" inp))
                (slot-set 4 (json-get-str "c4" inp))
                (near/store-bytes "F" (li-add (json-get-str "f" inp) (json-get-str "nf" inp)))
                (near/store-bytes "G" (li-add (json-get-str "g" inp) (json-get-str "p" inp)))
                (near/store-bytes "AB" (li-add (json-get-str "na" inp) (json-get-str "u" inp)))
                (near/store-bytes "PB" (li-sub (json-get-str "np" inp) (json-get-str "d" inp)))
                (near/return (json-get-str "r" inp))))))
      (near/return "forbidden")))

(define (pay-b)
  (if (self-call?)
      (let ((inp (near/input)))
        (let ((res (near/promise_result 0)))
          (if (< (str-length res) 1)
              (begin
                (slot-set 0 (li-add (slot-get 0) (json-get-str "x0" inp)))
                (slot-set 1 (li-add (slot-get 1) (json-get-str "x1" inp)))
                (slot-set 2 (li-add (slot-get 2) (json-get-str "x2" inp)))
                (slot-set 3 (li-add (slot-get 3) (json-get-str "x3" inp)))
                (slot-set 4 (li-add (slot-get 4) (json-get-str "x4" inp)))
                (near/store-bytes "SHT" (li-add (bz "SHT") (json-get-str "s" inp)))
                (near/store-bytes (sh-key (json-get-str "w" inp))
                  (li-add (bz (sh-key (json-get-str "w" inp))) (json-get-str "s" inp)))
                (near/store-bytes "AB" (li-add (bz "AB") (json-get-str "a" inp)))
                (near/store-bytes "PB" (li-add (bz "PB") (json-get-str "b" inp)))
                (near/store-bytes (acm-ck-key (json-get-str "w" inp)) (json-get-str "kc" inp))
                (near/store-bytes (acm-bal-key (json-get-str "w" inp)) (json-get-str "bc" inp))
                (near/return "wd-failed"))
              (if (str= (json-get-str "g" inp) "1")
                  (let ((p (near/promise_batch_create (near/load-bytes "TOKB"))))
                    (begin
                      (near/promise_batch_action_function_call p "ft_transfer"
                        (json-set (json-set "{}" "receiver_id" (json-get-str "r" inp)) "amount" (json-get-str "b" inp))
                        "0" 40000000000000)
                      (near/store-bytes (acm-bal-key (json-get-str "w" inp)) "0")
                      (near/promise_return p)))
                  (near/return "wd-ok")))))
      (near/return "forbidden")))

(define (withdraw4)
  (let ((inp (near/input)))
    (let ((who (near/predecessor_account_id)))
      (let ((sh (json-get-str "sh" inp)))
        (if (str= (num-ok? sh) "0")
            "bad-sh"
        (if (< (li-cmp sh "1") 0)
            "no-shares"
        (if (< (li-cmp (bz (sh-key who)) sh) 0)
            (str-cat "insufficient-shares-have:" (bz (sh-key who)))
            (let ((ab (bz "AB")) (pb (bz "PB")) (sht (bz "SHT")))
              (let ((kc (acm-get (acm-ck-key who)))
                    (bc (acm-get (acm-bal-key who)))
                    (owed (acm-settle who)))
                (let ((out-a (u128-muldiv sh ab sht))
                      (out-b (u128-muldiv sh pb sht)))
                  (let ((tot-a (li-add out-a (li-add bc owed))))
                    (let ((o0 (slot-get 0)) (o1 (slot-get 1)) (o2 (slot-get 2))
                          (o3 (slot-get 3)) (o4 (slot-get 4)))
                      (begin
                        (near/store-bytes "SHT" (li-sub sht sh))
                        (let ((sh-left (li-sub (bz (sh-key who)) sh)))
                          (if (str= sh-left "0")
                              (near/storage_remove (sh-key who))
                              (near/store-bytes (sh-key who) sh-left)))
                        (near/store-bytes "AB" (li-sub ab out-a))
                        (near/store-bytes "PB" (li-sub pb out-b))
                        (slot-set 0 (u128-muldiv o0 (bz "PB") pb))
                        (slot-set 1 (u128-muldiv o1 (bz "PB") pb))
                        (slot-set 2 (u128-muldiv o2 (bz "PB") pb))
                        (slot-set 3 (u128-muldiv o3 (bz "PB") pb))
                        (slot-set 4 (u128-muldiv o4 (bz "PB") pb))
                        (let ((args (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set "{}" "r" who) "b" out-b) "w" who) "s" sh) "a" out-a) "g" "1") "x0" (li-sub o0 (slot-get 0))) "x1" (li-sub o1 (slot-get 1))) "x2" (li-sub o2 (slot-get 2))) "x3" (li-sub o3 (slot-get 3))) "x4" (li-sub o4 (slot-get 4))) "kc" kc) "bc" bc)))
                          (if (str= tot-a "0")
                              (let ((p (near/promise_batch_create (near/load-bytes "TOKB"))))
                                (begin
                                  (near/promise_batch_action_function_call p "ft_transfer"
                                    (json-set (json-set "{}" "receiver_id" who) "amount" out-b)
                                    "0" 40000000000000)
                                  (near/promise_return
                                    (near/promise_then p (near/current_account_id) "pay_b"
                                      (json-set args "g" "0") "0" 60000000000000))
                                  "wd-queued"))
                              (let ((p (near/promise_batch_create (near/load-bytes "TOKA"))))
                                (begin
                                  (near/promise_batch_action_function_call p "ft_transfer"
                                    (json-set (json-set "{}" "receiver_id" who) "amount" tot-a)
                                    "0" 40000000000000)
                                  (near/promise_return
                                    (near/promise_then p (near/current_account_id) "pay_b"
                                      args "0" 60000000000000))
                                  "wd-queued")))))))))))))))))
'''

TRANSFORMS = [
    ('(define (sh-key a) (str-cat "SH:" a))',
     '(define (sh-key a) (str-cat "SH:" a))\n'
     '(define TWO64 "18446744073709551616")\n'
     + ACCRUAL_DEFS,
     "accrual defs"),

    ('        (near/store-bytes "SHT" "0")\n',
     '        (near/store-bytes "SHT" "0")\n'
     '        (near/store-bytes "G" "0")\n',
     "init G"),

    ('''            (near/store-bytes "PB" (li-add (bz "PB") amt))
            (near/store-bytes (sh-key (json-get-str "sender_id" (near/input)))''',
     '''            (near/store-bytes "PB" (li-add (bz "PB") amt))
            (acm-settle (json-get-str "sender_id" (near/input)))
            (near/store-bytes (acm-ck-key (json-get-str "sender_id" (near/input))) (acm-get "G"))
            (near/store-bytes (sh-key (json-get-str "sender_id" (near/input)))''',
     "H2 deposit"),
    # H1 swap: bind f1, grow G, carry exact increment "p" for reversal.

    ('(export "ft_on_transfer" ft-on-transfer4)',
     '(export "ft_on_transfer" ft-on-transfer4)\n'
     '(export "accrual_state" accrual-state)\n'
     '(export "claim" acm-claim)\n'
     '(export "withdraw" withdraw4)\n'
     '(export "init" pool-init4)',
     "exports"),
    ('              (begin\n                (slot-set 0 (car (cdr (cdr (cdr s0)))))\n                (slot-set 1 (car (cdr (cdr (cdr s1)))))\n                (slot-set 2 (car (cdr (cdr (cdr s2)))))\n                (slot-set 3 (car (cdr (cdr (cdr s3)))))\n                (slot-set 4 (car (cdr (cdr (cdr s4)))))\n                (near/store-bytes "F"\n                  (li-add (fee-get) (car (cdr (cdr s4)))))\n                (near/store-bytes "DY" (car (cdr s4)))\n                (near/store-bytes "REM" (car s4))\n                (str-cat (car (cdr s4)) "/" (car s4))',
     '              ;; M1a: the wave NEVER stores during the walk -- the\n              ;; whole result returns as a tuple (rem dy fee u0..u4);\n              ;; commits happen in pay_out success (or dy=0 inline).\n              ;; A hidden store here would corrupt the next tx after\n              ;; a reversal (the fail leg restores absolutes only).\n              (list (car s4) (car (cdr s4)) (car (cdr (cdr s4)))\n                (car (cdr (cdr (cdr s0)))) (car (cdr (cdr (cdr s1))))\n                (car (cdr (cdr (cdr s2)))) (car (cdr (cdr (cdr s3))))\n                (car (cdr (cdr (cdr s4))))',
     "M1a swap wave-tuple return (full deferral)"),

    ('                    (pool-swap amt)\n                    (let ((dy (bz "DY")) (rem (bz "REM")))\n                      (if (< (li-cmp dy min) 0)\n                          ;; slippage guard: restore pre-walk state, refund all A\n                          (begin\n                            (slot-set 0 s0v) (slot-set 1 s1v) (slot-set 2 s2v)\n                            (slot-set 3 s3v) (slot-set 4 s4v)\n                            (near/store-bytes "F" f0)\n                            (near/return amt))\n                          (begin\n                            (near/store-bytes "AB" (li-add (bz "AB") (li-sub amt rem)))\n                            (near/store-bytes "PB" (li-sub (bz "PB") dy))\n                            (if (str= dy "0")\n                                (near/return rem)\n                                (let ((p (near/promise_batch_create (near/load-bytes "TOKB"))))\n                                  (let ((args (json-set (json-set (json-set (json-set (json-set "{}" "r" rem) "a" amt)\n                                                    "u" (li-sub amt rem)) "d" dy) "f" (li-sub (fee-get) f0))))\n                                    (let ((args (json-set (json-set (json-set (json-set (json-set args\n                                                    "c0" (li-sub s0v (slot-get 0))) "c1" (li-sub s1v (slot-get 1)))\n                                                    "c2" (li-sub s2v (slot-get 2))) "c3" (li-sub s3v (slot-get 3)))\n                                                    "c4" (li-sub s4v (slot-get 4)))))\n                                      (begin\n                                        (near/promise_batch_action_function_call p "ft_transfer"\n                                          (json-set (json-set "{}" "receiver_id" sender) "amount" dy)\n                                          "0" 40000000000000)\n                                        (near/promise_return\n                                          (near/promise_then p (near/current_account_id) "pay_out"\n                                            args "0" 5000000000000)))))))))))))',
     '                    ;; M1a write-coalesce: NOTHING stored pre-promise. The\n                    ;; walk runs in registers; the wave (slots + F/G/AB/PB)\n                    ;; rides the callback args and commits in pay_out\'s\n                    ;; success branch. Slippage/zero refunds are FREE (no\n                    ;; restore stores -- nothing was ever written).\n                    (let ((w (pool-swap amt)))\n                      (let ((rem (car w)) (dy (car (cdr w))))\n                        (if (< (li-cmp dy min) 0)\n                            (near/return amt)\n                            (let ((f1 (car (cdr (cdr w))))\n                                  (g0 (acm-get "G"))\n                                  (ab0 (bz "AB"))\n                                  (pb0 (bz "PB")))\n                              (let ((inc (acm-grow f1)))\n                                (if (str= dy "0")\n                                    (begin\n                                      (slot-set 0 (car (cdr (cdr (cdr w))))) (slot-set 1 (car (cdr (cdr (cdr (cdr w))))))\n                                      (slot-set 2 (car (cdr (cdr (cdr (cdr (cdr w))))))) (slot-set 3 (car (cdr (cdr (cdr (cdr (cdr (cdr w))))))))\n                                      (slot-set 4 (car (cdr (cdr (cdr (cdr (cdr (cdr (cdr w)))))))))\n                                      (near/store-bytes "F" (li-add f0 f1))\n                                      (near/store-bytes "G" (li-add g0 inc))\n                                      (near/store-bytes "AB" (li-add ab0 (li-sub amt rem)))\n                                      (near/store-bytes "PB" (li-sub pb0 dy))\n                                      (near/return rem))\n                                    (let ((p (near/promise_batch_create (near/load-bytes "TOKB"))))\n                                      (let ((args (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set (json-set "{}" "r" rem) "a" amt) "u" (li-sub amt rem)) "d" dy) "f" f0) "nf" f1) "p" inc) "g" g0) "na" ab0) "np" pb0) "c0" (car (cdr (cdr (cdr w))))) "c1" (car (cdr (cdr (cdr (cdr w)))))) ))\n                                        (begin\n                                          (near/promise_batch_action_function_call p "ft_transfer"\n                                            (json-set (json-set "{}" "receiver_id" sender) "amount" dy)\n                                            "0" 40000000000000)\n                                          (near/promise_return\n                                            (near/promise_then p (near/current_account_id) "pay_out"\n                                              args "0" 5000000000000))))))))))))))',
     "M1a defer wave-commit into pay_out"),
]


def paren_balance(src):
    """Net () balance outside string literals."""
    bal = 0; instr = False; i = 0
    while i < len(src):
        c = src[i]
        if instr:
            if c == '\\':
                i += 2; continue
            if c == '"':
                instr = False
        else:
            if c == '"':
                instr = True
            elif c == '(':
                bal += 1
            elif c == ')':
                bal -= 1
        i += 1
    return bal


def build_source():
    parts = []
    for frag in ("limb_math.lisp", "clmm_core.lisp", "clmm_pool.lisp", "clmm_pool_v4.lisp"):
        with open(os.path.join(LIB, frag)) as f:
            parts.append(f";; ===== from lib/{frag} (verbatim) =====\n" + f.read())
    src = "\n".join(parts)
    for anchor, repl, name in TRANSFORMS:
        n = src.count(anchor)
        assert n == 1, f"anchor {name!r}: expected 1 occurrence, found {n}"
        src = src.replace(anchor, repl)
    # H1-H4 whole-tail splice: promise/withdraw section replaced by
    # PAY_SECTION (Jean's semantics + H hooks, balance-checked constant)
    assert paren_balance(PAY_SECTION) == 0, 'PAY_SECTION drifted unbalanced'
    a = src.index('(define (pay-out)')
    b = src.index('(define (msg-min')
    assert 0 < a < b, 'splice anchors out of order'
    src = src[:a] + PAY_SECTION + '\n' + src[b:]

    bal = paren_balance(src)
    assert bal == 0, f"paren balance broken after transforms: net {bal}"
    return src


# ------------------------------------------------------------- scenario
def build_scenario(p):
    st = []
    LP1, LP2, LP3, TR, ADMIN = (
        "lp1.accrual.test.near", "lp2.accrual.test.near", "lp3.accrual.test.near",
        "trader.accrual.test.near", "admin.accrual.test.near")
    POOL, TKA, TKB = ("pool.accrual.test.near", "tka.accrual.test.near",
                      "tkb.accrual.test.near")

    def tx(method, contains=None, contract=POOL, args=None, **kw):
        s = {"contract": contract, "method": method}
        if args is not None: s["args"] = args
        s.update(kw)
        if contains is not None: s["contains"] = contains
        st.append(s)

    def view(method, contains, args=None, contract=POOL):
        tx(method, contains, contract=contract, args=args or {},
           **{"as": ADMIN, "view": True})

    def deposit(lp, amt):
        # fee_split idiom: direct ft_on_transfer with predecessor=TOKEN;
        # sender_id comes from args (the pool reads it from input)
        tx("ft_on_transfer", None,
           args={"sender_id": lp, "amount": amt, "msg": ""},
           **{"as": TKB, "predecessor": TKB})
        p.add_b_liq(lp, amt)

    def swap(tr, amt):
        tx("ft_on_transfer", None,
           args={"sender_id": tr, "amount": amt, "msg": "swap:1"},
           **{"as": TKA, "predecessor": TKA})
        p.swap_a(tr, amt, "1")

    # -- setup: init ONLY (no floats yet: reversal pins run against an
    #    empty pool token float, so payout promises trap and the fail legs
    #    of pay_out / pay_b actually fire) --
    tx("init", "ok", args={"toka": TKA, "tokb": TKB}, **{"as": ADMIN})

    # -- deposits (H2) --
    deposit(LP1, "6000000")
    deposit(LP2, "4000000")
    tx("accrual_state",
       f"G:0 F:0 SHT:{p.SHT} AB:0 PB:{p.PB} SH:4000000 CK:0 BAL:0",
       args={"lp": LP2})

    # -- REVERSAL PIN 1 (H1 fail leg): swap with pool B float = 0.
    #    Swap executes, dy > 0, B payout promise TRAPS, pay-out fail leg
    #    restores slots + F + G + AB + PB and refunds the full amount. --
    fx = p.swap_a(TR, "1000000", "1", fail_b=True)
    tx("ft_on_transfer", "1000000",
       args={"sender_id": TR, "amount": "1000000", "msg": "swap:1"},
       **{"as": TKA, "predecessor": TKA})
    view("accrual_state",
         f"G:{p.G} F:{p.F} SHT:{p.SHT} AB:{p.AB} PB:{p.PB} "
         f"SH:6000000 CK:0 BAL:0", {"lp": LP1})
    assert p.G == "0" and p.F == "0" and p.AB == "0", "H1 reversal state"
    tx("claim", "0", **{"as": LP1})  # post-reversal claim pays nothing

    # -- floats minted AFTER the reversal pins (payouts clear from here) --
    tx("mint", "10000000", contract=TKB, args={"to": POOL, "amount": "10000000"},
       **{"as": ADMIN})
    p.floatB = "10000000"

    # -- swap 1 (H1): G grows by f1*2^64//SHT --
    swap(TR, "1000000")
    view("accrual_state", f"G:{p.G} F:{p.F} SHT:{p.SHT} AB:{p.AB} PB:{p.PB}",
         {"lp": LP2})
    c1 = p.claim(LP1)
    tx("claim", c1, **{"as": LP1})
    view("accrual_state", f"CK:{p.CK[LP1]} BAL:{p.BAL[LP1]}", {"lp": LP1})

    # -- late LP3 joins mid-stream (skim pin): entry checkpoint = current G --
    deposit(LP3, "10")
    view("accrual_state", f"SH:{p.SH[LP3]} CK:{p.CK[LP3]}", {"lp": LP3})
    assert p.CK[LP3] == p.G, "LP3 entry checkpoint must be current G"

    # -- swap 2, remaining claims (LP3's claim = skim only) --
    swap(TR, "1000000")
    c2 = p.claim(LP2)
    tx("claim", c2, **{"as": LP2})
    c1b = p.claim(LP1)
    tx("claim", c1b, **{"as": LP1})
    c3 = p.claim(LP3)
    tx("claim", c3, **{"as": LP3})
    assert int(c3) * 1000000 < int(c1), "SKIM: late LP must not share wave 1"
    view("accrual_state",
         f"SH:4000000 CK:{p.CK[LP2]} BAL:{p.BAL[LP2]}", {"lp": LP2})

    total_owed = int(p.BAL[LP1]) + int(p.BAL[LP2]) + int(p.BAL[LP3])
    total_fees = int(p.F)
    assert 0 < total_owed <= total_fees, f"SOLVENCY: {total_owed} > {total_fees}"
    print(f"mirror: F={p.F} G={p.G} owed1={c1}+{c1b} owed2={c2} owed3={c3} "
          f"solvency {total_owed}<={total_fees}")

    # -- REVERSAL PIN 2 (H3+H4 fail leg): withdraw with pool A float = 0.
    #    tot_a = out_a + bc + owed > 0 -> A payout promise TRAPS -> pay_b
    #    fail leg restores SHT/SH/AB/PB/slots + ck:=kc, bal:=bc. --
    wf = p.withdraw(LP1, "6000000", fail_a=True)
    tx("withdraw", "wd-failed", args={"sh": "6000000"}, **{"as": LP1})
    view("accrual_state",
         f"SHT:{p.SHT} AB:{p.AB} PB:{p.PB} SH:6000000 "
         f"CK:{p.CK[LP1]} BAL:{p.BAL[LP1]}", {"lp": LP1})
    assert p.SH[LP1] == "6000000" and p.CK[LP1] == wf["kc"]

    # -- float A minted after the reversal pin; real withdraw (H3+H4) --
    tx("mint", "10000000", contract=TKA, args={"to": POOL, "amount": "10000000"},
       **{"as": ADMIN})
    p.floatA = "10000000"
    w = p.withdraw(LP1, "6000000")
    assert int(w["tot_a"]) == int(w["out_a"]) + int(w["bc"]) + int(w["owed"])
    tx("withdraw", None, args={"sh": "6000000"}, **{"as": LP1})
    view("accrual_state",
         f"AB:{p.AB} PB:{p.PB} SH:0", {"lp": LP1})

    # -- token-ledger pins (real FT legs paid everything) --
    view("ft_balance_of", str(p.tk["A"][LP1]), contract=TKA,
         args={"account": LP1})
    view("ft_balance_of", str(p.tk["B"][LP1]), contract=TKB,
         args={"account": LP1})
    view("ft_balance_of", str(p.tk["B"][TR]), contract=TKB,
         args={"account": TR})
    view("ft_balance_of", str(p.floatA), contract=TKA,
         args={"account": POOL})
    view("ft_balance_of", str(p.floatB), contract=TKB,
         args={"account": POOL})

    scenario = {
        "name": "clmm-accrual-mount-v4",
        "manifest": (f"{POOL}=../../../contracts/accrual-mount/v4-pool/target/v4pool.wasm,"
                     f"{TKA}=../../../contracts/escrow/ft/target/ft.wasm,"
                     f"{TKB}=../../../contracts/escrow/ft/target/ft.wasm"),
        "state": "state.bin",
        "steps": st,
    }
    os.makedirs(SCN_DIR, exist_ok=True)
    with open(SCN, "w") as f:
        json.dump(scenario, f, indent=1)
        f.write("\n")
    print(f"wrote {SCN} ({len(st)} steps)")


if __name__ == "__main__":
    src = build_source()
    os.makedirs(OUT_DIR, exist_ok=True)
    with open(OUT, "w") as f:
        f.write(src)
    print(f"wrote {OUT} ({len(src)} bytes, {len(TRANSFORMS)} transforms applied)")
    with open(os.path.join(V4, "near.json"), "w") as f:
        json.dump({"name": "v4pool", "src": "src/main.built.lisp",
                   "account": "pool.accrual.test.near", "network": "local",
                   "output": "target/v4pool.wasm"}, f, indent=1)
        f.write("\n")
    build_scenario(Pool())
