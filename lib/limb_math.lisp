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
