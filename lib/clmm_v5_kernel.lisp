;; ═══════════════════════════════════════════════════════════════════════
;; lib/clmm_v5_kernel.lisp — CLMM v5 integer kernel, limb-stack port (M2)
;;
;; Parity target: scripts/clmm_v5_ref.py (the ONLY oracle — the parity
;; driver imports it directly). Equivalences proven here in comments:
;;
;;  • v5-isqrt-dd — MSB-first binary (digit-pair) square root, exact for
;;    all n ≥ 0 by construction (no seed, no convergence): invariant after
;;    each pair is rem = (n >> 2k) − root², root = isqrt(n >> 2k) — the
;;    classic long-division sqrt. ⇒ v5-isqrt-dd = isqrt_u128 = math.isqrt.
;;    Zero-alloc; used by the 1M-case checksum batch.
;;
;;  • v5-isqrt-newton — the REF's exact algorithm, verbatim structure:
;;    n<2 early return, x = 2^ceil(bl/2) over-seed, y=(x+n//x)>>1,
;;    break when y >= x, then the two repair loops. Single-op path
;;    (implemented in the dispatch tail on the string helpers).
;;
;;  • S_for_* closed forms replace the ref's binary searches (values
;;    identical — proofs):
;;    dy_out(t)=floor(L·t/2^64) nondecreasing; dy_out ≤ dy ⟺ t ≤
;;    (2^64(dy+1)−1)//L for L>0 ⇒ with t* = (2^64(dy+1)−1)//L:
;;      S_for_dy    = L=0 ? 2^127 : min(2^127, S + t*)   [largest]
;;      S_for_dy_in = L=0 ? 1    : max(1, S − t*)        [smallest]
;;    dx_out ≤ dx ⟺ L(S−S2) < (dx+1)·S·S2 ⟺ L·S+1 ≤ S2·D with
;;    D = L+(dx+1)·S ⟺ S2 ≥ ceil((LS+1)/D) ⇒
;;      S_for_dx_out = L=0 ? 1 : max(1, min(S, ceil((LS+1)/D)))
;;    dx_in nondecreasing with dx_in(L,S,S)=0 ⇒ for dx ≥ 0
;;      S_for_dx     = S  (condition already holds at lo=S)
;;    ceil(a/D) = floor((a+D−1)/D) exactly.
;;
;;  • S(p) = isqrt(floor(1.0001^p · 2^128)): repeated squaring at scale
;;    D=10^108 (12 limbs — division by D = limb drop, exact). B+ =
;;    10001·10^104 = floor(1.0001·D) EXACT; B− = floor(10000·D/10001)
;;    (1 ulp at 10^112 scale). ≤ ~40 muldrops; each truncation ≤ 1 ulp at
;;    D-scale; v ∈ [10^−34.74, 10^34.74] ⇒ total relative error
;;    ≤ 2^40·10^−73.3 < 10^−60 ⇒ absolute error of the pre-floor real
;;    v·2^128 ≪ 0.5 for every tested p (edges incl. ±800000 + 200k PRNG
;;    cases vs the Decimal(130) oracle; v·2^128's distance to ℤ is ≥
;;    10^-3.2M theoretically, O(0.1) typically). p<0 runs the same chain
;;    on B− so the final ⌊·2^128/D⌋ is always doubling+drop.
;;
;; Zero-alloc batch path conventions:
;;  • Buffers are raw tagged values. Working widths (limbs): 6 u128
;;    values, 7 dd-sqrt internals, 10 products/divmod, 16 pow-chain, 32
;;    pow products. Buffers ≥ width+2 limbs (limb-add/sub guards need
;;    (max+1)·4 ≤ alloc; limb-mul needs (la+lb)·4 ≤ alloc).
;;  • All values zero-padded to working width; ops use FIXED widths.
;;    No strings / str->limbs / buf-alloc inside batch loops.
;;  • loop bodies keep (recur …) in direct tail position only.
;; ═══════════════════════════════════════════════════════════════════════
(memory 1024)

;; ── section 0: decimal-string helpers (single-op path; allocs OK) ──────

(define (v5-li-add a b)
  (let ((A (str->limbs a)) (B (str->limbs b)) (r (buf-alloc 64)))
    (limbs->str r (limb-add (car A) (car B) r (car (cdr A)) (car (cdr B))))))

(define (v5-li-sub a b)
  ;; requires a >= b (limb-sub traps on underflow — caller contract)
  (let ((A (str->limbs a)) (B (str->limbs b)) (r (buf-alloc 64)))
    (limbs->str r (limb-sub (car A) (car B) r (car (cdr A)) (car (cdr B))))))

(define (v5-li-cmp a b)
  (let ((A (str->limbs a)) (B (str->limbs b)))
    (limb-cmp (car A) (car B) (car (cdr A)) (car (cdr B)))))

;; floor(a·b / c), c > 0 — string interface (single-op path)
(define (v5-muldiv a b c)
  (u128-muldiv a b c))

;; ── section 1: buffer micro-helpers (zero-alloc) ────────────────────────

(define (v5-zero! b k)
  (loop ((j 0))
    (if (>= j k)
        0
        (begin (limb-set! b j 0) (recur (+ j 1))))))

;; zero limbs [s, s+k) of b
(define (v5-zero-off! b s k)
  (loop ((j 0))
    (if (>= j k)
        0
        (begin (limb-set! b (+ s j) 0) (recur (+ j 1))))))

(define (v5-copy! a b k)
  (loop ((j 0))
    (if (>= j k)
        0
        (begin (limb-set! b j (limb-get a j)) (recur (+ j 1))))))

;; copy limbs [s, s+k) of a into [0,k) of b
(define (v5-copy-off! a s b k)
  (loop ((j 0))
    (if (>= j k)
        0
        (begin (limb-set! b j (limb-get a (+ s j))) (recur (+ j 1))))))

;; b <- floor(b/2) over k limbs (in place)
(define (v5-halve! b k)
  (loop ((j (- k 1)) (carry 0))
    (if (< j 0) 0
        (let ((v (+ (limb-get b j) (* carry 1000000000))))
          (begin (limb-set! b j (/ v 2)) (recur (- j 1) (mod v 2)))))))

;; b <- b·2 over k limbs — carry flows low->high (walk j upward)
(define (v5-double! b k)
  (loop ((j 0) (carry 0))
    (if (>= j k) 0
        (let ((v (+ (* (limb-get b j) 2) carry)))
          (begin (limb-set! b j (mod v 1000000000))
                 (recur (+ j 1) (/ v 1000000000)))))))

(define (v5-strip b k)
  (loop ((j (- k 1)))
    (if (< j 0)
        0
        (if (> (limb-get b j) 0) (+ j 1) (recur (- j 1))))))

;; set b (6 limbs) from i64 v ≥ 0 — 3-limb safe for all v < 2^62
(define (v5-u64! b v)
  (begin
    (v5-zero! b 6)
    (limb-set! b 0 (mod v 1000000000))
    (limb-set! b 1 (mod (/ v 1000000000) 1000000000))
    (limb-set! b 2 (/ v 1000000000000000000)) 0))

;; LSB-first bit string of b into byte buffer s; returns bit length.
;; DESTROYS b (halves in place). b width k.
(define (v5-bitlen! b k s)
  (loop ((j 0))
    (if (<= (v5-strip b k) 0) j
        (begin
          (buf-set! s j (+ 48 (mod (limb-get b 0) 2)))
          (v5-halve! b k)
          (recur (+ j 1))))))

;; ── section 2: mod-M fold + PRNG (M = 10^36 − 11; 10^36 ≡ 11 mod M) ────
;; Fold round (when u > 10^36, i.e. strip > 4): u = 11·(u>>10^36) + lo.
;; Residual u ∈ [M, 10^36) handled by ≤ 2 trailing subtractions.

;; u <- (u + w) mod M ; w width wk ≤ 7 limbs. Scratch hi(8+), h11(8+),
;; t(9+), one11 (1-limb, value 11). u buffer ≥ 10 limbs. M = 4-limb buf.
(define (v5-modm! u w wk hi h11 t one11 M)
  (begin
    (v5-zero! t 9)
    (v5-copy! w t wk)
    (limb-add u t u 8 8)
    (loop ((g 0))
      (if (>= g 10) 0
          (if (<= (v5-strip u 8) 4) 0
              (begin
                (v5-zero! hi 8)
                (v5-copy-off! u 4 hi (- (v5-strip u 8) 4))
                (v5-zero-off! u 4 4)
                (limb-mul hi one11 h11 4 1)
                (limb-add u h11 u 8 8)
                (recur (+ g 1))))))
    (if (> (limb-cmp u M 8 4) 0) (limb-sub u M u 8 4) 0)
    (if (> (limb-cmp u M 8 4) 0) (limb-sub u M u 8 4) 0)))

;; xb (4 limbs) <- (xb² + 1) mod M. Scratch P(12), u(10), hi(10), h11(10).
(define (v5-prng-step! xb P u hi h11 one11 M)
  (begin
    (v5-zero! P 12)
    (limb-mul xb xb P 4 4)
    (v5-copy-off! P 4 u 4)          ;; u = x² >> 10^36
    (limb-mul u one11 h11 4 1)      ;; h11 = 11·hi  (≤ 1.1e37)
    (v5-copy-off! P 0 u 4)          ;; u = lo
    (limb-add u h11 u 8 8)          ;; u = 11·hi + lo
    (limb-set! hi 0 1)
    (limb-add u hi u 8 8)           ;; +1
    (loop ((g 0))
      (if (>= g 6) 0
          (if (<= (v5-strip u 8) 4) 0
              (begin
                (v5-zero! hi 8)
                (v5-copy-off! u 4 hi (- (v5-strip u 8) 4))
                (v5-zero-off! u 4 4)
                (limb-mul hi one11 h11 4 1)
                (limb-add u h11 u 8 8)
                (recur (+ g 1))))))
    (if (> (limb-cmp u M 8 4) 0) (limb-sub u M u 8 4) 0)
    (if (> (limb-cmp u M 8 4) 0) (limb-sub u M u 8 4) 0)
    (v5-copy! u xb 4)))

;; ── section 3: isqrt — dd (digit-pair, exact, zero-alloc) ───────────────
;; nb (k limbs) in; scratch nbs (k), s (bytes ≥ 8·k); rem/root/t0/t1
;; (9-limb bufs, width 7). Result in root (zero-padded).

(define (v5-isqrt-dd! nb k nbs s rem root t0 t1)
  (begin
    (v5-copy! nb nbs k)
    (v5-zero! rem 7)
    (v5-zero! root 7)
    (let ((bl (v5-bitlen! nbs k s)))
      ;; groups from the top: i = ceil(bl/2)−1 … 0; group i = bits
      ;; (2i+1, 2i); the top group is a single bit when bl is odd.
      (loop ((i (- (/ (+ bl (mod bl 2)) 2) 1)))
        (if (< i 0) 0
            (begin
              ;; rem = 4·rem + pair(2i+1, 2i)
              (limb-add rem rem rem 7 7)
              (limb-add rem rem rem 7 7)
              (limb-set! t1 0
                (+ (if (< (+ (* 2 i) 1) bl) (* 2 (- (buf-get s (+ (* 2 i) 1)) 48)) 0)
                   (- (buf-get s (* 2 i)) 48)))
              (limb-add rem t1 rem 7 7)
              ;; root = 2·root ; t0 = 2·root + 1
              (limb-add root root root 7 7)
              (limb-add root root t0 7 7)
              (limb-set! t1 0 1)
              (limb-add t0 t1 t0 7 7)
              (if (>= (limb-cmp rem t0 7 7) 0)
                  (begin
                    (limb-sub rem t0 rem 7 7)
                    (limb-set! t1 0 1)
                    (limb-add root t1 root 7 7))
                  0)
              (recur (- i 1))))))))

;; ── section 4: muldrop + pow chain (scale D = 10^108 = 12 limbs) ───────

;; out (working 16, buffer ≥ 18) <- floor(a·b / 10^108); P 40-limb buf.
(define (v5-muldrop! a b P out)
  (begin
    (v5-zero! P 32)
    (limb-mul a b P 16 16)
    (v5-copy-off! P 12 out 16)))

;; acc/R 16-limb (buffers ≥ 18): acc <- base^q at scale D (acc starts D,
;; R starts B). Exact repeated square-and-multiply.
(define (v5-pow-run! q acc R P)
  (loop ((qq q))
    (if (< qq 1) 0
        (begin
          (if (> (mod qq 2) 0) (v5-muldrop! acc R P acc) 0)
          (let ((nq (/ qq 2)))
            (begin
              (if (> nq 0) (v5-muldrop! R R P R) 0)
              (recur nq)))))))

;; N (working 12, buffer ≥ 14) <- floor(acc·2^128 / 10^108); t ≥ 26 limbs
(define (v5-pow-final! acc t N)
  (begin
    (v5-zero! t 24)
    (v5-copy! acc t 16)
    (loop ((j 0))
      (if (>= j 128) 0 (begin (v5-double! t 24) (recur (+ j 1)))))
    (v5-copy-off! t 12 N 12)))

;; ── section 5: divmod (bit-walk; zero-alloc) ────────────────────────────
;; q <- floor(a/b), r <- a mod b ; a/b/q/r 10-limb (buffers ≥ 12).
;; scratch nbs (10, destroyed), s (bytes ≥ 90), one (1-limb).

(define (v5-divmod! a b q r nbs s one)
  (begin
    (v5-zero! q 12)
    (v5-zero! r 10)
    (v5-copy! a nbs 10)
    (let ((bl (v5-bitlen! nbs 10 s)))
      (loop ((i (- bl 1)))
        (if (< i 0) 0
            (begin
              (limb-add r r r 10 10)
              (if (> (- (buf-get s i) 48) 0)
                  (begin (limb-set! one 0 1) (limb-add r one r 10 1))
                  0)
              (limb-add q q q 10 10)
              (if (>= (limb-cmp r b 10 10) 0)
                  (begin
                    (limb-sub r b r 10 10)
                    (limb-set! one 0 1)
                    (limb-add q one q 10 1))
                  0)
              (recur (- i 1))))))))

;; ── section 6: segment math + fee (buffer forms, zero-alloc) ───────────

;; out <- (L·(S2−S)) >> 64 ; requires S2 >= S. d(8), P(14), out(12+)
(define (v5-dy! L S S2 d P out)
  (begin
    (limb-sub S2 S d 6 6)
    (v5-zero! P 12)
    (limb-mul L d P 6 6)
    (v5-copy! P out 10)
    (loop ((j 0))
      (if (>= j 64) 0 (begin (v5-halve! out 10) (recur (+ j 1)))))))

;; out <- L·(S−S2) // (S·S2) ; requires S >= S2 >= 1.
(define (v5-dx! L S S2 d P Q out q r nbs s one)
  ;; d = S2 − S (up-convention; callers swap args for dx_out)
  (begin
    (limb-sub S2 S d 6 6)
    (v5-zero! P 12)
    (limb-mul L d P 6 6)
    (v5-zero! Q 12)
    (limb-mul S S2 Q 6 6)
    (v5-divmod! P Q out r nbs s one)))

;; fee: out <- (amt·bps + 9999) // 10000. t (14-limb scratch).
(define (v5-fee! amt bps out t)
  (begin
    (v5-zero! t 12)
    (limb-mul amt bps t 6 6)
    (v5-zero! out 10)
    (limb-set! out 0 9999)
    (limb-add t out t 12 12)
    (v5-zero! out 10)
    (loop ((j (- (v5-strip t 12) 1)) (rr 0))
      (if (< j 0) 0
          (let ((cur (+ (* rr 1000000000) (limb-get t j))))
            (begin
              (limb-set! out j (/ cur 10000))
              (recur (- j 1) (mod cur 10000))))))))


;; small-divisor division: out <- floor(a / d), d i64 in [1, 9999].
;; limb walk top->down: cur = rr·10^9 + limb < 10^13 (i64). a/out ≤ 12+ limbs.
(define (v5-sdiv! a d out k)
  (begin
    (v5-zero! out 12)
    (loop ((j (- k 1)) (rr 0))
      (if (< j 0) 0
          (let ((cur (+ (* rr 1000000000) (limb-get a j))))
            (begin
              (limb-set! out j (/ cur d))
              (recur (- j 1) (mod cur d))))))))

;; ═══════════════ batch case drivers ════════════════════════════════════
;; One case each; called from the TCO batch loops in the dispatch tail.
;; sumb folds via v5-modm!; first 8 results stored into rvec (8×7 limbs).

(define (v5-isqrt-case! xb P u hi h11 one11 M sumb
                        hi2 h11b t9 one11b s rem root t0 t1 nbs rvec i)
  (begin
    (v5-isqrt-dd! xb 6 nbs s rem root t0 t1)
    (v5-modm! sumb root 7 hi2 h11b t9 one11b M)
    (if (< i 8)
        (loop ((jj 0))
          (if (>= jj 7) 0
              (begin
                (limb-set! rvec (+ (* i 7) jj) (limb-get root jj))
                (recur (+ jj 1)))))
        0)
    (v5-prng-step! xb P u hi h11 one11 M)))

(define (v5-spow-case! p acc R P t24 N nbs2 s2 rem root t0 t1
                       sumb hi2 h11b t9 one11b M D Bpos Bneg)
  (begin
    (v5-copy! D acc 16)
    (v5-copy! Bpos R 16)
    (if (< p 0) (v5-copy! Bneg R 16) 0)
    (v5-pow-run! (if (< p 0) (- 0 p) p) acc R P)
    (v5-pow-final! acc t24 N)
    (v5-isqrt-dd! N 12 nbs2 s2 rem root t0 t1)
    (v5-modm! sumb root 7 hi2 h11b t9 one11b M)))
