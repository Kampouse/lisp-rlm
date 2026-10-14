;; ===== from lib/limb_math.lisp (verbatim) =====
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

;; ===== from lib/clmm_core.lisp (verbatim) =====
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

;; ===== from lib/clmm_pool.lisp (verbatim) =====
;; lib/clmm_pool.lisp — stateful point-ladder CLMM pool v1 on NEAR storage.
;; State (decimal-string values, raw storage_write/read via near/*-bytes):
;;   "S0".."S4" → remaining token-A liquidity per price slot
;;   "F"        → accumulated fee (token-A units)
;; Ladder config (compiled in): prices 0.98/0.99/1.00/1.01/1.02 B-per-A,
;; initial La 1000/2000/5000/3000/1500, fee 30 bps. v1: swap A→B only,
;; single liquidity pool, no per-user shares (next: NEP-141 legs + LPs).

;; stateful step: like clmm-step but returns (dx' dy' fee' la_left)
(define (clmm-step4 st pnum pden la fee-bps)
  (let ((dx (car st)) (dy (car (cdr st))) (fe (car (cdr (cdr st)))))
    (let ((lt (li-cmp dx la)))
      (let ((used (if (< lt 1) dx la))
            (rem  (if (< lt 1) "0" (li-sub dx la)))
            (left (if (< lt 1) (li-sub la dx) "0")))
        (list rem
              (li-add dy (u128-muldiv used pnum pden))
              (li-add fe (u128-muldiv used (to-string fee-bps) "10000"))
              left)))))

(define (slot-key i) (str-cat "S" (to-string i)))
(define (slot-get i) (near/load-bytes (slot-key i)))
(define (slot-set i v) (near/store-bytes (slot-key i) v))

(define (pool-init)
  (begin
    (slot-set 0 "1000") (slot-set 1 "2000") (slot-set 2 "5000")
    (slot-set 3 "3000") (slot-set 4 "1500")
    (near/store-bytes "F" "0")))

(define (fee-get) (near/load-bytes "F"))

;; walk all 5 slots; persist decremented liquidity + fees
(define (pool-swap amt)
  (let ((st (list amt "0" "0")))
    (let ((s0 (clmm-step4 st "98" "100" (slot-get 0) 30)))
      (let ((s1 (clmm-step4 (list (car s0) (car (cdr s0)) (car (cdr (cdr s0))))
                            "99" "100" (slot-get 1) 30)))
        (let ((s2 (clmm-step4 (list (car s1) (car (cdr s1)) (car (cdr (cdr s1))))
                              "100" "100" (slot-get 2) 30)))
          (let ((s3 (clmm-step4 (list (car s2) (car (cdr s2)) (car (cdr (cdr s2))))
                                "101" "100" (slot-get 3) 30)))
            (let ((s4 (clmm-step4 (list (car s3) (car (cdr s3)) (car (cdr (cdr s3))))
                                  "102" "100" (slot-get 4) 30)))
              (begin
                (slot-set 0 (car (cdr (cdr (cdr s0)))))
                (slot-set 1 (car (cdr (cdr (cdr s1)))))
                (slot-set 2 (car (cdr (cdr (cdr s2)))))
                (slot-set 3 (car (cdr (cdr (cdr s3)))))
                (slot-set 4 (car (cdr (cdr (cdr s4)))))
                (near/store-bytes "F"
                  (li-add (fee-get) (car (cdr (cdr s4)))))
                (near/store-bytes "DY" (car (cdr s4)))
                (near/store-bytes "REM" (car s4))
                (str-cat (car (cdr s4)) "/" (car s4))))))))))

(define (pool-state)
  (str-cat "s0:" (slot-get 0) " s1:" (slot-get 1) " s2:" (slot-get 2)
           " s3:" (slot-get 3) " s4:" (slot-get 4) " F:" (fee-get)))

;; ===== from lib/clmm_pool_v4.lisp (verbatim) =====
;; lib/clmm_pool_v4.lisp — book-ownership LP pool over the v2 ladder.
;; Model: LPs deposit B (token-B via ft_on_transfer, predecessor == TOKB).
;; First deposit bootstraps the ladder (fixed shape 1000:2000:5000:3000:1500,
;; later deposits proportional to current slot fill). Swaps (token-A side,
;; predecessor == TOKA, msg "swap") walk the ladder as v2; ALL A received
;; (amt - rem) accrues to "AB" — LP revenue. Withdraw burns shares, pays out
;; BOTH sides: out_A = sh·AB//SHT (promise on TOKA) chained into a callback
;; that pays out_B = sh·PB//SHT (promise on TOKB). Slots shrink by the
;; withdrawn B fraction → sellable-B ≤ pool-B invariant preserved (floored).
;; Storage: S0..S4 (A-capacity, start ZERO), F (fee display, A), TOKA/TOKB,
;; AB (A book), PB (B book ≡ B balance), SHT, SH:<a>.
;; Share price (A+B per share) rises with swap volume — fees included in AB.

(define (bz k) (let ((v (near/load-bytes k))) (if (nil? v) "0" v)))
(define (sh-key a) (str-cat "SH:" a))
(define TWO64 "18446744073709551616")

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
  ;; returns the increment: store-bytes itself returns "" -- a value that
  ;; rides the reversal args MUST be a real string (H1 exactness).
  (if (str= (bz "SHT") "0")
      "0"
      (let ((inc (u128-muldiv f TWO64 (bz "SHT"))))
        (begin
          (near/store-bytes "G" (li-add (acm-get "G") inc))
          inc))))

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


(define (self-call?) (str= (near/predecessor_account_id) (near/current_account_id)))

(define (num-ok? s)
  ;; <=40 digits, ASCII only — OOB-parse & u128-scale guard (Cetus-class)
  (if (> (str-length s) 40)
      "0"
      (loop ((i 0))
        (if (>= i (str-length s))
            "1"
            (let ((d (- (byte-at s i) 48)))
              (if (or (< d 0) (> d 9))
                  "0"
                  (recur (+ i 1))))))))

(define (pool-init4)
  ;; one-shot: a bound TOKA means already initialized — state untouched
  (if (nil? (near/load-bytes "TOKA"))
      (begin
        (slot-set 0 "0") (slot-set 1 "0") (slot-set 2 "0")
        (slot-set 3 "0") (slot-set 4 "0")
        (near/store-bytes "F" "0")
        (near/store-bytes "AB" "0")
        (near/store-bytes "PB" "0")
        (near/store-bytes "SHT" "0")
        (near/store-bytes "G" "0")
        (near/store-bytes "TOKA" (json-get-str "toka" (near/input)))
        (near/store-bytes "TOKB" (json-get-str "tokb" (near/input)))
        "ok")
      "already-initialized"))

(define (slot-sum)
  (li-add (li-add (li-add (li-add (slot-get 0) (slot-get 1)) (slot-get 2)) (slot-get 3)) (slot-get 4)))

;; slot i += (amt·base_i//tot)·100//pnum_i — base = shape (first) or slot (later)
(define (slot-widen i base amt tot pnum)
  (slot-set i (li-add (slot-get i)
    (u128-muldiv (u128-muldiv amt base tot) "100" pnum))))

(define (add-b-liq amt)
  ;; returns the unused-B refund for ft_on_transfer: "0" on success.
  ;; Validate BEFORE mutating: dust / zero-share / bad-amount all refund untouched.
  (if (< (li-cmp amt "10") 0)
      amt  ;; below min deposit (10): refund, no state write (sybil/DoS guard)
      (let ((sh (if (str= (bz "SHT") "0")
                amt
                (u128-muldiv amt (bz "SHT") (bz "PB")))))
    (if (str= sh "0")
        amt  ;; full refund — dust deposit floors to zero shares
        (let ((tot (slot-sum)))
          (begin
            (if (str= tot "0")
                (begin (slot-widen 0 "1000" amt "12500" "98")
                       (slot-widen 1 "2000" amt "12500" "99")
                       (slot-widen 2 "5000" amt "12500" "100")
                       (slot-widen 3 "3000" amt "12500" "101")
                       (slot-widen 4 "1500" amt "12500" "102"))
                (begin (slot-widen 0 (slot-get 0) amt tot "98")
                       (slot-widen 1 (slot-get 1) amt tot "99")
                       (slot-widen 2 (slot-get 2) amt tot "100")
                       (slot-widen 3 (slot-get 3) amt tot "101")
                       (slot-widen 4 (slot-get 4) amt tot "102")))
            (near/store-bytes "SHT" (li-add (bz "SHT") sh))
            (near/store-bytes "PB" (li-add (bz "PB") amt))
            (acm-settle (json-get-str "sender_id" (near/input)))
            (near/store-bytes (acm-ck-key (json-get-str "sender_id" (near/input))) (acm-get "G"))
            (near/store-bytes (sh-key (json-get-str "sender_id" (near/input)))
              (li-add (bz (sh-key (json-get-str "sender_id" (near/input)))) sh))
            "0"))))))

(define (pay-out)
  (if (self-call?)
      (let ((inp (near/input)))
        (let ((res (near/promise_result 0)))
          (if (< (str-length res) 1)
              (begin
                (slot-set 0 (li-add (slot-get 0) (json-get-str "c0" inp)))
                (slot-set 1 (li-add (slot-get 1) (json-get-str "c1" inp)))
                (slot-set 2 (li-add (slot-get 2) (json-get-str "c2" inp)))
                (slot-set 3 (li-add (slot-get 3) (json-get-str "c3" inp)))
                (slot-set 4 (li-add (slot-get 4) (json-get-str "c4" inp)))
                (near/store-bytes "F" (li-sub (fee-get) (json-get-str "f" inp)))
                (near/store-bytes "G" (li-sub (acm-get "G") (json-get-str "p" inp)))
                (near/store-bytes "AB" (li-sub (bz "AB") (json-get-str "u" inp)))
                (near/store-bytes "PB" (li-add (bz "PB") (json-get-str "d" inp)))
                (near/return (json-get-str "a" inp)))
              (near/return (json-get-str "r" inp)))))
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

(define (msg-min msg)
  ;; "swap" → "0"; "swap:NNN" → NNN
  (if (< (str-length msg) 6)
      "0"
      (str-substring msg 5 (str-length msg))))

(define (ft-on-transfer4)
  (let ((inp (near/input)))
    (let ((sender (json-get-str "sender_id" inp)))
      (let ((amt (json-get-str "amount" inp)))
        (if (str= (num-ok? amt) "0")
            (near/return amt)
            (if (str= (near/predecessor_account_id) (near/load-bytes "TOKB"))
            (near/return (add-b-liq amt))
            (if (str= (near/predecessor_account_id) (near/load-bytes "TOKA"))
                (let ((min (msg-min (json-get-str "msg" inp))))
                  (if (str= (num-ok? min) "0")
                      (near/return amt)
                  (let ((f0 (fee-get))
                        (s0v (slot-get 0)) (s1v (slot-get 1)) (s2v (slot-get 2))
                        (s3v (slot-get 3)) (s4v (slot-get 4)))
                    (pool-swap amt)
                    (let ((dy (bz "DY")) (rem (bz "REM")))
                      (if (< (li-cmp dy min) 0)
                          ;; slippage guard: restore pre-walk state, refund all A
                          (begin
                            (slot-set 0 s0v) (slot-set 1 s1v) (slot-set 2 s2v)
                            (slot-set 3 s3v) (slot-set 4 s4v)
                            (near/store-bytes "F" f0)
                            (near/return amt))
                          (begin
                            (near/store-bytes "AB" (li-add (bz "AB") (li-sub amt rem)))
                            (near/store-bytes "PB" (li-sub (bz "PB") dy))
                            (if (str= dy "0")
                                (near/return rem)
                                (let ((p (near/promise_batch_create (near/load-bytes "TOKB"))))
                                  (let ((args (json-set (json-set (json-set (json-set (json-set (json-set "{}" "r" rem) "a" amt)
                                                    "u" (li-sub amt rem)) "d" dy) "f" (li-sub (fee-get) f0)) "p" (acm-grow (li-sub (fee-get) f0)))))
                                    (let ((args (json-set (json-set (json-set (json-set (json-set args
                                                    "c0" (li-sub s0v (slot-get 0))) "c1" (li-sub s1v (slot-get 1)))
                                                    "c2" (li-sub s2v (slot-get 2))) "c3" (li-sub s3v (slot-get 3)))
                                                    "c4" (li-sub s4v (slot-get 4)))))
                                      (begin
                                        (near/promise_batch_action_function_call p "ft_transfer"
                                          (json-set (json-set "{}" "receiver_id" sender) "amount" dy)
                                          "0" 40000000000000)
                                        (near/promise_return
                                          (near/promise_then p (near/current_account_id) "pay_out"
                                            args "0" 5000000000000)))))))))))))
                (near/return amt))))))))

(export "ft_on_transfer" ft-on-transfer4)
(export "accrual_state" accrual-state)
(export "claim" acm-claim)
(export "withdraw" withdraw4)
(export "init" pool-init4)
(export "pay_out" pay-out)
(export "pay_b" pay-b)
