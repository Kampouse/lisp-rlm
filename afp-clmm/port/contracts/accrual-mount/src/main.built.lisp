; ASSEMBLED by gen_scenario.py — edit main.lisp, not this file.
; ---- lib/limb_math.lisp ----
;; limb_math.lisp — big-number math on limb buffers (base 10^9, little-endian)
;;
;; The limb builtins (limb-add/sub/cmp/mul/get/set!) are VALUE arithmetic on
;; base-10^9 limbs — limb values are always < 10^9 and carry at 10^9 (the
;; factorial/fmt tests rely on this; pad9 printing is the convention).
;; Everything below uses ONLY value ops (add/sub/cmp/mul) — no binary bit
;; reads — so it stays correct in whatever base the builtins normalize to.
;;
;; API (all top-level, TCO loops only):
;;   (limb-zero k)           fresh all-zero k-limb buffer
;;   (u32-buf n)             1-limb buffer holding n (< 10^9 here)
;;   (limb-strip b l)        length w/o leading zero limbs (min 1)
;;   (str->limbs s)          decimal str → (buf len)   [value: ×10 accumulate]
;;   (limbs->str b l)        buffer → decimal str      [pad9 concatenation]
;;   (limb-halve b l)        → (buf len carry): floor(b/2), carry = b mod 2
;;   (limb-bits n ln)        MSB-first bit string of the VALUE ("1011…")
;;   (limb-divmod n ln d ld) → (q lq r lr): q=floor(n/d), r=n mod d; d≠0
;;   (u128-muldiv a b c)     decimal floor(a·b/c)
;;   (u128-mulmod a b m)     decimal (a·b) mod m
;;
;; Division = LSB halving walk (extract bits) + MSB-first shift-subtract
;; with value-doubling quotient. bits(n) iterations × ~4 value ops.

(define (limb-zero k)
  (let ((b (buf-alloc (* k 4))))
    (loop ((i 0))
      (if (>= i k) b
          (begin (limb-set! b i 0) (recur (+ i 1)))))))

(define (u32-buf n)
  (let ((b (buf-alloc 8)))
    (begin (limb-set! b 0 n) b)))

(define (limb-strip b l)
  (loop ((ll l))
    (if (> ll 1)
        (if (= (limb-get b (- ll 1)) 0) (recur (- ll 1)) ll)
        ll)))

(define (str->limbs s)
  ;; acc = acc*10 + digit. Returns (buf len).
  (loop ((i 0) (acc (u32-buf 0)) (la 1))
    (if (>= i (str-length s))
        (list acc (limb-strip acc la))
        (let ((t1 (buf-alloc 64)) (ten (u32-buf 10))
              (t2 (buf-alloc 64)) (db (u32-buf (- (byte-at s i) 48))))
          (let ((lt (limb-mul acc ten t1 la 1)))
            (let ((l2 (limb-add t1 db t2 lt 1)))
              (recur (+ i 1) t2 l2)))))))

(define (limbs-pad9 s)
  (let ((n (- 9 (str-length s))))
    (if (<= n 0) s (str-cat (str-substring "000000000" 0 n) s))))

(define (limbs->str b l)
  ;; base-10^9 limbs → decimal: top limb plain, lower padded to 9 digits
  (loop ((j (- l 1)) (acc ""))
    (if (>= j 0)
        (if (= j (- l 1))
            (recur (- j 1) (to-string (limb-get b j)))
            (recur (- j 1) (str-cat acc (limbs-pad9 (to-string (limb-get b j))))))
        acc)))

;; halve: walk limbs top→bottom, v = limb + 10^9×carry (carry∈{0,1};
;; v ≤ 2×10^9 < 2^31 ✓), limb' = v/2, next carry = v mod 2.
;; returns (buf len carry) — carry is the bit shifted out.
(define (limb-halve b l)
  (let ((out (buf-alloc 64)))
    (loop ((k (- (limb-strip b l) 1)) (carry 0))
      (if (>= k 0)
          (let ((v (+ (limb-get b k) (* carry 1000000000))))
            (begin
              (limb-set! out k (/ v 2))
              (recur (- k 1) (mod v 2))))
          (list out (limb-strip out (limb-strip b l)) carry)))))

;; MSB-first bit string of the VALUE: halve repeatedly; each carry-out is
;; the next bit toward the LSB. Prepending builds MSB-first for free.
(define (limb-bits n ln)
  (loop ((m n) (lm (limb-strip n ln)) (acc ""))
    (if (and (= lm 1) (= (limb-get m 0) 0))
        acc
        (let ((h (limb-halve m lm)))
          (let ((nm (car h)) (nlm (car (cdr h))) (bit (car (cdr (cdr h)))))
            (recur nm nlm (str-cat (to-string bit) acc)))))))

;; ---- divmod ----
;; step at bit i (MSB→LSB): r = r*2 + bit; if r ≥ d: r -= d, qbit=1 else 0;
;; q = q*2 + qbit. All value ops — base-agnostic.
(define (limb-divstep r t1 one z q d dd bit lr lq ldd)
  (let ((l1 (limb-add r r t1 lr lr)))
    (let ((l2 (if (= bit 1) (limb-add t1 one t1 l1 1) l1)))
      (if (>= (limb-cmp t1 d l2 ldd) 0)
          (let ((l3 (limb-sub t1 d r l2 ldd)))
            (let ((lq2 (limb-add q q t1 lq lq)))
              (let ((lq3 (limb-add t1 one q lq2 1)))
                (list l3 lq3))))
          (let ((l3 (limb-add t1 z r l2 1)))
            (let ((lq2 (limb-add q q q lq lq)))
              (list l3 lq2)))))))

(define (limb-divmod n ln d ld)
  ;; q = floor(n/d), r = n mod d (d ≠ 0). Returns (q lq r lr).
  ;; limb-bits returns the MSB-FIRST bit string, so consume i = 0 → len-1.
  ;; Scratch sizing: q ≤ n (limb-wise), and divstep's t1 doubles as the
  ;; q-doubling scratch — both sized off lnn (numerator limbs), NOT ldd.
  (let ((lnn (limb-strip n ln))
        (ldd (limb-strip d ld))
        (bs (limb-bits n ln)))
    (let ((q (limb-zero (+ lnn 2)))
          (r (limb-zero (+ ldd 4)))
          (t1 (limb-zero (+ lnn 4)))
          (one (u32-buf 1))
          (z (u32-buf 0)))
      (loop ((i 0) (lr 1) (lq 1))
        (if (>= i (str-length bs))
            (list q (limb-strip q lq) r (limb-strip r lr))
            (let ((st (limb-divstep r t1 one z q d ldd
                                    (- (byte-at bs i) 48) lr lq ldd)))
              (recur (+ i 1) (car st) (car (cdr st)))))))))

;; ---- decimal-string convenience wrappers ----

(define (u128-muldiv a b c)
  ;; floor(a·b / c); decimal strings, c ≠ "0"
  (let ((A (str->limbs a)) (B (str->limbs b)) (C (str->limbs c)))
    (let ((ab (buf-alloc 64)))
      (let ((lab (limb-mul (car A) (car B) ab (car (cdr A)) (car (cdr B)))))
        (let ((res (limb-divmod ab lab (car C) (car (cdr C)))))
          (limbs->str (car res) (car (cdr res))))))))

(define (u128-mulmod a b m)
  ;; (a·b) mod m; decimal strings, m ≠ "0"
  (let ((A (str->limbs a)) (B (str->limbs b)) (M (str->limbs m)))
    (let ((ab (buf-alloc 64)))
      (let ((lab (limb-mul (car A) (car B) ab (car (cdr A)) (car (cdr B)))))
        (let ((res (limb-divmod ab lab (car M) (car (cdr M)))))
          (limbs->str (car (cdr (cdr res))) (car (cdr (cdr (cdr res))))))))))

; ---- lib/clmm_core.lisp ----
;; lib/clmm_core.lisp — point-ladder CLMM swap core on top of limb_math.
;; Model (DCL-v1-style, piecewise-constant marginal price): a pool is an
;; ascending ladder of price slots; slot i sells token-B per token-A at
;; integer ratio pnum_i/pden_i, absorbing up to La_i of token-A. A swap
;; walks the ladder consuming slot liquidity until input is exhausted.
;; All heavy math = fuzz-verified u128-muldiv/mulmod (arbitrary precision).

;; --- string-space helpers over limb buffers (normalize via strip) ---
(define (li-add a b)
  (let ((A (str->limbs a)) (B (str->limbs b)) (r (buf-alloc 64)))
    (limbs->str r (limb-add (car A) (car B) r (car (cdr A)) (car (cdr B))))))

(define (li-sub a b)
  (let ((A (str->limbs a)) (B (str->limbs b)) (r (buf-alloc 64)))
    (limbs->str r (limb-sub (car A) (car B) r (car (cdr A)) (car (cdr B))))))

(define (li-cmp a b)
  (let ((A (str->limbs a)) (B (str->limbs b)))
    (limb-cmp (car A) (car B) (car (cdr A)) (car (cdr B)))))

;; --- one ladder step ---
;; st = (dx dy fee) decimal strings; consumes min(dx, la) of token-A at
;; price pnum/pden (token-B out per token-A in), fee = used·fee_bps/10000.
(define (clmm-step st pnum pden la fee-bps)
  (let ((dx (car st)) (dy (car (cdr st))) (fe (car (cdr (cdr st)))))
    (let ((lt (li-cmp dx la)))
      (let ((used (if (< lt 1) dx la))
            (rem  (if (< lt 1) "0" (li-sub dx la))))
        (let ((dy2 (li-add dy (u128-muldiv used pnum pden)))
              (fe2 (li-add fe (u128-muldiv used (to-string fee-bps) "10000"))))
          (list rem dy2 fe2))))))

; ---- harness ----
;; accrual-mount harness — the proven fee-ledger core, exercised at pool scale.
;; Mount design: port/accrual-mount.md (PS section).
;;
;; ONE global per-share fee accumulator in 2^64 fixed point:
;;   PS += f * 2^64 // SHT          (fee wave, H1)
;;   CK:<lp> := PS                  (deposit, H2)
;;   owed = (PS - CK:<lp>) * sh // 2^64   (claim, H3)
;; This IS accruef (Aggregate_Accrual.thy) with constant cap Lf = sh:
;; theorem accrue_within_charged_book => total owed <= total fees,
;; floored, fail-closed. Late LPs checkpoint at entry => no skim.
;;
;; Harness shorthands vs the real pool (deliberate, under test is the
;; LEDGER): m-deposit takes lp + amt directly (v4 mints PB-proportionally
;; at ft_on_transfer); scenario numbers are generator-computed
;; (gen_scenario.py) — zero hand-typed expectations.
;;
;; u128-muldiv(a, b, d) = a*b//d: muldiv(f, TWO64, SHT) and
;; muldiv(dPS, sh, TWO64) are exactly the two divs above. Sibling-call
;; discipline: one call per let* binding (compiler clobber rule).

(define (sput k v) (near/storage_set k v))
(define (sget k) (default (near/storage_get k) "0"))
(define (fail m) (near/log m) (near/abort m))
(define (num-ok? s)
  (if (> (str-length s) 40) "0"
      (loop ((i 0))
        (if (>= i (str-length s)) "1"
            (let ((d (- (byte-at s i) 48)))
              (if (or (< d 0) (> d 9)) "0" (recur (+ i 1))))))))

(define TWO64 "18446744073709551616")

;; H1: fee wave f -> PS += f*TWO64//SHT
(define (m-fee)
  (let* ((inp (near/input))
         (fs (json-get-str "f" inp))
         (ok (num-ok? fs))
         (g (if (str= ok "0") (fail "ERR_NUM") "ok"))
         (f fs)
         (sht (sget "SHT"))
         (z (if (str= sht "0") (fail "ERR_NO_LPS") "ok"))
         (inc (u128-muldiv f TWO64 sht))
         (ps (sget "PS"))
         (ps2 (li-add ps inc)))
    (begin (sput "PS" ps2) (near/return (str-cat "PS:" ps2)))))

;; H2: deposit mints amt shares, checkpoint at current PS
(define (m-deposit)
  (let* ((inp (near/input))
         (lp (json-get-str "lp" inp))
         (amts (json-get-str "amt" inp))
         (ok (num-ok? amts))
         (g0 (if (str= ok "0") (fail "ERR_NUM") "ok"))
         (amt amts)
         (ten (li-cmp amt "10"))
         (g1 (if (< ten 0) (fail "ERR_MIN") "ok"))
         (ps (sget "PS"))
         (ckk (str-cat "CK:" lp))
         (shk (str-cat "SH:" lp))
         (sh0 (sget shk))
         (sh1 (li-add sh0 amt))
         (sht (sget "SHT"))
         (sht1 (li-add sht amt)))
    (begin
      (sput shk sh1)
      (sput "SHT" sht1)
      (sput ckk ps)
      (near/return (str-cat "SHT:" sht1)))))

;; H3: settle owed = (PS - CK)*sh//TWO64, credit BAL, checkpoint := PS
(define (m-claim)
  (let* ((inp (near/input))
         (lp (json-get-str "lp" inp))
         (ckk (str-cat "CK:" lp))
         (shk (str-cat "SH:" lp))
         (blk (str-cat "BAL:" lp))
         (ck (sget ckk))
         (ps (sget "PS"))
         (dps (li-sub ps ck))
         (sh (sget shk))
         (owed (u128-muldiv dps sh TWO64))
         (b0 (sget blk))
         (b1 (li-add b0 owed)))
    (begin
      (sput blk b1)
      (sput ckk ps)
      (near/return (str-cat "BAL:" lp "=" b1)))))

(define (m-view)
  (let* ((sht (sget "SHT"))
         (ps (sget "PS"))
         (b1 (sget "BAL:lp1.test.near"))
         (b2 (sget "BAL:lp2.test.near"))
         (b3 (sget "BAL:lp3.test.near"))
         (t (li-add b1 (li-add b2 b3))))
    (near/return (str-cat "SHT:" sht " PS:" ps " OWEDSUM:" t))))

(define (m-init)
  (begin
    (sput "SHT" "0")
    (sput "PS" "0")
    (near/return "SHT:0")))

(export "m-init" m-init)
(export "m-fee" m-fee)
(export "m-deposit" m-deposit)
(export "m-claim" m-claim)
(export "m-view" m-view)
