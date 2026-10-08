;; ═══════════════════════════════════════════════════════════════════════
;; scripts/clmm_v5_dispatch_tail.lisp — _run dispatch + batch loops (M2)
;;
;; Concatenated AFTER lib/clmm_v5_kernel.lisp (TASK-M2 build line).
;; Ops (JSON in / 📄 JSON out via _run):
;;   single: isqrt isqrt_dd spow dy dx dyi dxo sfdy sfdx sfdyi sfdxo fee
;;   batch:  isqrt_batch spow_batch dy_batch dx_batch dyi_batch dxo_batch
;;           sfdy_batch sfdx_batch sfdyi_batch sfdxo_batch fee_batch
;;           (seed,n) → {"n":N,"sum":S,"s0".."s7":R} (raw numeric JSON)
;;
;; Batch loop shape (TCO-critical): body = (if (>= i n) EXIT (recur (step i)))
;; — recur DIRECT in else; per-case work runs inside the step fn (an ARG
;; expression). Buffers are free-vars threaded by the compiler.
;;
;; Shared PRNG (lisp == python): x <- (x²+1) mod (10^36−11). Streams:
;;   isqrt:  n_i = x_i (eval THEN step); x_0 = seed.
;;   others: step first, derive case from new x.
;;   spow:   p = (x mod 1600001) − 800000
;;   dy/dx:  S = x1+1; i even: S2 = S + (x2>>100); else S2 = S
;;   dyi/dxo:S = x1+1; i even: S2 = max(1, S−(x2>>100)); else S2 = S
;;   sfdy/sfdyi/sfdxo: S = x1+1; dy|dx = x2>>74
;;   fee:    amt = x mod 10^18; bps = [0,1,100,400,2000,9999,10000][i%7]
;;   L = [0, 1, 10^18, 5·10^17][i%4]
;; ═══════════════════════════════════════════════════════════════════════

;; ── single-op adapters ─────────────────────────────────────────────────

(define (v5-str6 s)
  ;; 12-limb zeroed buffer holding decimal string s (≤ ~30 digits)
  (let ((b (buf-alloc 48)) (A (str->limbs s)))
    (begin (v5-zero! b 12) (v5-copy! (car A) b (car (cdr A))) b)))

(define (v5-buf-str b)
  (limbs->str b (if (> (v5-strip b 12) 0) (v5-strip b 12) 1)))

(define (v5-op-isqrt-dd nstr)
  (let ((nb (v5-str6 nstr)) (nbs (buf-alloc 48)) (s (buf-alloc 160))
        (rem (buf-alloc 48)) (root (buf-alloc 48)) (t0 (buf-alloc 48))
        (t1 (buf-alloc 48)))
    (begin (v5-isqrt-dd! nb 6 nbs s rem root t0 t1) (v5-buf-str root))))

(define (v5-li-halve s)
  (let ((A (str->limbs s)) (o (buf-alloc 64)))
    (let ((h (limb-halve (car A) (car (cdr A)))))
      (limbs->str (car h) (car (cdr h))))))

;; REF-exact Newton isqrt (string path)
(define (v5-op-isqrt-newton nstr)
  (if (< (v5-li-cmp nstr "2") 0)
      nstr
      (let ((A (str->limbs nstr)))
        (let ((bl (str-length (limb-bits (car A) (car (cdr A))))))
          (let ((seed-k (/ (+ bl 1) 2)))
            (let ((x0 (loop ((j 0) (acc "1"))
                         (if (>= j seed-k) acc
                             (recur (+ j 1) (v5-li-add acc acc))))))
              (v5-newton-run nstr x0)))))))

(define (v5-newton-run nstr x)
  ;; y = (x + n//x) >> 1  — muldiv is floor(a·b/c): n//x needs b="1", c=x
  (let ((y (v5-li-halve (v5-li-add x (v5-muldiv nstr "1" x)))))
    (if (>= (v5-li-cmp y x) 0)
        (v5-newton-down nstr x)
        (v5-newton-run nstr y))))

(define (v5-newton-down nstr x)
  (if (> (v5-li-cmp (v5-muldiv x x "1") nstr) 0)
      (v5-newton-down nstr (v5-li-sub x "1"))
      (v5-newton-up nstr x)))

(define (v5-newton-up nstr x)
  (if (<= (v5-li-cmp (v5-muldiv (v5-li-add x "1") (v5-li-add x "1") "1")
                     nstr) 0)
      (v5-newton-up nstr (v5-li-add x "1"))
      x))

(define (v5-op-spow p)
  (let ((acc (buf-alloc 88)) (R (buf-alloc 88)) (P (buf-alloc 176))
        (t24 (buf-alloc 120)) (N (buf-alloc 64)) (nbs2 (buf-alloc 64))
        (s2 (buf-alloc 256)) (rem (buf-alloc 48)) (root (buf-alloc 48))
        (t0 (buf-alloc 48)) (t1 (buf-alloc 48)) (D (buf-alloc 88))
        (Bp (buf-alloc 88)) (Bn (buf-alloc 88)))
    (begin
      (v5-zero! D 22) (limb-set! D 12 1)
      (v5-zero! Bp 22)
      (let ((BP (str->limbs "1000100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000")))
        (v5-copy! (car BP) Bp (car (cdr BP))))
      (v5-zero! Bn 22)
      (let ((BA (str->limbs "10000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000")) (BO (buf-alloc 64)))
        (begin (v5-sdiv! (car BA) 10001 BO 13) (v5-copy! BO Bn 13)))
      (v5-copy! D acc 16)
      (v5-copy! Bp R 16)
      (if (< p 0) (v5-copy! Bn R 16) 0)
      (v5-pow-run! (if (< p 0) (- 0 p) p) acc R P)
      (v5-pow-final! acc t24 N)
      (v5-isqrt-dd! N 12 nbs2 s2 rem root t0 t1)
      (v5-buf-str root))))

(define (v5-op-dy Lstr Sstr S2str)
  (let ((L (v5-str6 Lstr)) (S (v5-str6 Sstr)) (S2 (v5-str6 S2str))
        (d (buf-alloc 32)) (P (buf-alloc 64)) (out (buf-alloc 64)))
    (begin (v5-dy! L S S2 d P out) (v5-buf-str out))))

(define (v5-op-dx Lstr Sstr S2str)
  (let ((L (v5-str6 Lstr)) (S (v5-str6 Sstr)) (S2 (v5-str6 S2str))
        (d (buf-alloc 32)) (P (buf-alloc 64)) (Q (buf-alloc 64))
        (out (buf-alloc 64)) (q (buf-alloc 64)) (r (buf-alloc 64))
        (nbs (buf-alloc 64)) (s (buf-alloc 400)) (one (buf-alloc 8)))
    (begin (v5-dx! L S S2 d P Q out q r nbs s one) (v5-buf-str out))))

;; t* = (2^64·(dy+1) − 1) // L  into q. dyB: ≥4 zero-padded limbs holding
;; dy ≥ 0 (driver contract dy < 2^63, string-parsed — str->num caps at
;; 2^60 so dy arrives as a buffer). L must be > 0 (caller checks).
(define (v5-tstar! dyB L tA tB tC q r nbs s one)
  (begin
    (v5-zero! tA 12)
    (v5-copy! dyB tA 4)
    (limb-set! one 0 1)
    (limb-add tA one tA 12 1)
    (v5-zero! tB 12) (limb-set! tB 0 1)
    (loop ((j 0))
      (if (>= j 64) 0 (begin (v5-double! tB 10) (recur (+ j 1)))))
    (v5-zero! tC 20)
    (limb-mul tA tB tC 6 10)
    (limb-set! one 0 1)
    (limb-sub tC one tC 10 1)
    (v5-zero! q 10) (v5-zero! r 10)
    (v5-divmod! tC L q r nbs s one)))

;; S_for_dy: S ≥ 2^127 → S (ref lo≥hi, loop no-run) ; else
;; L=0 → 2^127 ; else min(2^127, S + t*)
(define (v5-op-sfdy Lstr Sstr dystr)
  (let ((L (v5-str6 Lstr)) (S (v5-str6 Sstr)) (dyB (v5-str6 dystr))
        (C127 (v5-str6 "170141183460469231731687303715884105728"))
        (tA (buf-alloc 80)) (tB (buf-alloc 80)) (tC (buf-alloc 88))
        (tD (buf-alloc 80)) (tE (buf-alloc 80)) (one (buf-alloc 8))
        (q (buf-alloc 64)) (r (buf-alloc 64)) (nbs (buf-alloc 64))
        (s (buf-alloc 400)))
    (begin
      (if (>= (limb-cmp S C127 12 12) 0)
          (begin (v5-zero! tE 12) (v5-copy! S tE 6))
          (if (<= (v5-strip L 12) 0)
              (begin (v5-zero! tE 12) (v5-copy! C127 tE 6))
              (begin
                (v5-tstar! dyB L tA tB tC q r nbs s one)
                (v5-zero! tD 12) (v5-copy! S tD 6)
                (v5-zero! tE 12) (v5-copy! q tE 10)
                (limb-add tD tE tE 10 10)
                (if (> (limb-cmp tE C127 10 10) 0)
                    (begin (v5-zero! tE 12) (v5-copy! C127 tE 6))
                    0))))
      (v5-buf-str tE))))

;; S_for_dy_in: L=0 → 1 ; else max(1, S − t*)
(define (v5-op-sfdyi Lstr Sstr dystr)
  (let ((L (v5-str6 Lstr)) (S (v5-str6 Sstr)) (dyB (v5-str6 dystr))
        (tA (buf-alloc 80)) (tB (buf-alloc 80)) (tC (buf-alloc 88))
        (tD (buf-alloc 80)) (tE (buf-alloc 80)) (one (buf-alloc 8))
        (q (buf-alloc 64)) (r (buf-alloc 64)) (nbs (buf-alloc 64))
        (s (buf-alloc 400)))
    (begin
      (if (<= (v5-strip L 12) 0)
          (begin (v5-zero! tE 12) (limb-set! tE 0 1) 0)
          (begin
            (v5-tstar! dyB L tA tB tC q r nbs s one)
            (v5-zero! tD 12) (v5-copy! S tD 6)
            (if (> (limb-cmp tD q 10 10) 0)
                (begin (limb-sub tD q tE 10 10) 0)
                (begin (v5-zero! tE 12) (limb-set! tE 0 1) 0))))
      (v5-buf-str tE))))

;; S_for_dx_out: max(1, min(S, ceil((L·S+1)/D))), D = L+(dx+1)·S.
;; ceil ≥ 1 and S ≥ 1 ⇒ result ≥ 1 (no clamp needed below 1).
(define (v5-op-sfdxo Lstr Sstr dxstr)
  (let ((L (v5-str6 Lstr)) (S (v5-str6 Sstr)) (dxB (v5-str6 dxstr))
        (tA (buf-alloc 80)) (tB (buf-alloc 80)) (tC (buf-alloc 88))
        (tD (buf-alloc 80)) (tE (buf-alloc 80)) (tF (buf-alloc 80))
        (one (buf-alloc 8)) (q (buf-alloc 64)) (r (buf-alloc 64))
        (nbs (buf-alloc 64)) (s (buf-alloc 400)))
    (begin
      (v5-zero! tA 20)
      (limb-mul L S tA 6 6)
      (limb-set! one 0 1)
      (limb-add tA one tA 10 1)
      (v5-zero! tC 12)
      (v5-copy! dxB tC 4)
      (limb-add tC one tC 12 1)
      (v5-zero! tB 12)
      (limb-add tB L tB 10 10)
      (v5-zero! tD 20)
      (limb-mul tC S tD 6 6)
      (v5-zero! tF 12)
      (v5-copy! tB tF 10)
      (limb-add tF tD tF 10 10)
      (v5-zero! tE 12)
      (v5-copy! tA tE 10)
      (limb-add tE tF tE 10 10)
      (limb-set! one 0 1)
      (limb-sub tE one tE 10 1)
      (v5-zero! q 10) (v5-zero! r 10)
      (v5-divmod! tE tF q r nbs s one)
      (v5-zero! tE 12)
      (v5-copy! q tE 10)
      (if (> (limb-cmp tE S 10 10) 0)
          (begin (v5-zero! tE 12) (v5-copy! S tE 6))
          0)
      (v5-buf-str tE))))

;; S_for_dx exact ref replication: the ref searches smallest S2 with
;; dx_in ≤ dx over a DECREASING predicate, so it returns S iff the FIRST
;; mid (S+2^127)//2 already satisfies dx_in ≤ dx (all later mids are
;; smaller → also satisfy), else climbs to 2^127. Verified equivalent on
;; 3448 ref cases incl. all edge classes. S ≥ 2^127 → S (loop no-run).
;; dx < 0 never sent by driver; → 2^127. Sign from the string: str->num
;; caps at 2^60 (tagged int), driver sends up to 2^62.
(define (v5-op-sfdx Lstr Sstr dxstr)
  (if (str= (str-substring dxstr 0 1) "-")
      (v5-buf-str (v5-str6 "170141183460469231731687303715884105728"))
      (let ((L (v5-str6 Lstr)) (S (v5-str6 Sstr)) (dxB (v5-str6 dxstr))
            (C127 (v5-str6 "170141183460469231731687303715884105728"))
            (tA (buf-alloc 80)) (tB (buf-alloc 80)) (tC (buf-alloc 88))
            (tD (buf-alloc 80)) (tE (buf-alloc 80)) (one (buf-alloc 8))
            (q (buf-alloc 64)) (r (buf-alloc 64)) (nbs (buf-alloc 64))
            (s (buf-alloc 400)))
        (begin
          (if (>= (limb-cmp S C127 12 12) 0)
              (begin (v5-zero! tE 12) (v5-copy! S tE 6))
              (begin
                ;; M = (S + 2^127) // 2
                (v5-zero! tA 12) (v5-copy! S tA 6)
                (limb-add tA C127 tA 10 10)
                (v5-halve! tA 10)
                ;; dx_in(L,S,M) = L·(M−S) // (S·M)
                (limb-sub tA S tB 10 10)
                (v5-zero! tC 20)
                (limb-mul L tB tC 6 10)
                (v5-zero! tD 20)
                (limb-mul S tA tD 6 10)
                (v5-zero! q 10) (v5-zero! r 10)
                (v5-divmod! tC tD q r nbs s one)
                (if (<= (limb-cmp q dxB 10 12) 0)
                    (begin (v5-zero! tE 12) (v5-copy! S tE 6))
                    (begin (v5-zero! tE 12) (v5-copy! C127 tE 6)))))
          (v5-buf-str tE)))))

(define (v5-op-fee amtstr bpsstr)
  (let ((amt (v5-str6 amtstr)) (bps (v5-str6 bpsstr))
        (out (buf-alloc 64)) (t (buf-alloc 64)))
    (begin (v5-fee! amt bps out t) (v5-buf-str out))))

;; ── batch case step fns (return i+1) ───────────────────────────────────

(define (v5b-isqrt-step xb n6 P u hi h11 one11 M sumb hi2 h11b t9
                        s rem root t0 t1 nbs rvec i)
  (begin
    (v5-zero! n6 6)
    (v5-copy! xb n6 4)
    (v5-isqrt-dd! n6 6 nbs s rem root t0 t1)
    (v5-modm! sumb root 7 hi2 h11b t9 one11 M)
    (if (< i 8)
        (loop ((jj 0))
          (if (>= jj 7) 0
              (begin
                (limb-set! rvec (+ (* i 7) jj) (limb-get root jj))
                (recur (+ jj 1)))))
        0)
    (v5-prng-step! xb P u hi h11 one11 M)
    (+ i 1)))

(define (v5b-spow-step xb x6 dv P u hi h11 one11 M sumb hi2 h11b t9
                       acc R P40 t24 N nbs2 s2 rem root t0 t1 D Bpos Bneg
                       q r nbs s one rvec i)
  (begin
    (v5-prng-step! xb P u hi h11 one11 M)
    (v5-zero! x6 12)
    (v5-copy! xb x6 4)
    (v5-zero! dv 12)
    (limb-set! dv 0 1600001)
    (v5-zero! q 10) (v5-zero! r 10)
    (v5-divmod! x6 dv q r nbs s one)
    (v5-copy! D acc 16)
    (v5-copy! Bpos R 16)
    (let ((p (- (limb-get r 0) 800000)))
      (begin
        (if (< p 0) (v5-copy! Bneg R 16) 0)
        (v5-pow-run! (if (< p 0) (- 0 p) p) acc R P40)
        (v5-pow-final! acc t24 N)
        (v5-isqrt-dd! N 12 nbs2 s2 rem root t0 t1)
        (v5-modm! sumb root 7 hi2 h11b t9 one11 M)
        (if (< i 8)
            (loop ((jj 0))
              (if (>= jj 7) 0
                  (begin
                    (limb-set! rvec (+ (* i 7) jj) (limb-get root jj))
                    (recur (+ jj 1)))))
            0)
        (+ i 1)))))

;; dy/dx (up=1, op=0 → dy, op=1 → dx) and dyi/dxo (up=0)
(define (v5b-seg-step up op L0 L1 L2 L3 xb n6 P u hi h11 one11 M
                      sumb hi2 h11b t9 d PQP PQQ out qr rr nbs s one
                      sB s2B hB Lh rvec i)
  (begin
    (v5-prng-step! xb P u hi h11 one11 M)       ;; x1
    (v5-zero! sB 6)
    (v5-copy! xb sB 4)
    (limb-set! one 0 1)
    (limb-add sB one sB 6 1)                    ;; S = x1 + 1
    (v5-prng-step! xb P u hi h11 one11 M)       ;; x2
    (v5-zero! hB 6)
    (v5-copy! xb hB 4)
    (loop ((j 0))
      (if (>= j 100) 0 (begin (v5-halve! hB 6) (recur (+ j 1)))))
    (v5-zero! s2B 6)
    (v5-copy! sB s2B 6)
    (if (= (mod i 2) 0)
        (if (> up 0)
            (begin (limb-add s2B hB s2B 6 6) 0)  ;; S2 = S + h
            (if (> (limb-cmp sB hB 6 6) 0)
                (begin (limb-sub sB hB s2B 6 6) 0) ;; S2 = S − h
                (begin (v5-zero! s2B 6) (limb-set! s2B 0 1) 0)))
        0)
    (v5-zero! Lh 6)
    (v5-copy! (if (= (mod i 4) 0) L0
               (if (= (mod i 4) 1) L1
               (if (= (mod i 4) 2) L2 L3))) Lh 6)
    (if (= op 0)
        (if (> up 0)
            (v5-dy! Lh sB s2B d PQP out)
            (v5-dy! Lh s2B sB d PQP out))
        (if (> up 0)
            (v5-dx! Lh sB s2B d PQP PQQ out qr rr nbs s one)
            (v5-dx! Lh s2B sB d PQP PQQ out qr rr nbs s one)))
    (v5-modm! sumb out 10 hi2 h11b t9 one11 M)
    (if (< i 8)
        (loop ((jj 0))
          (if (>= jj 7) 0
              (begin
                (limb-set! rvec (+ (* i 7) jj) (limb-get out jj))
                (recur (+ jj 1)))))
        0)
    (+ i 1)))

;; S_for batches: op 0 sfdy, 1 sfdyi, 2 sfdxo
(define (v5b-sfor-step op L0 L1 L2 L3 xb n6 P u hi h11 one11 M
                       sumb hi2 h11b t9 tA tB tC tD tE tF one
                       q r nbs s sB hB Lh C127 rvec i)
  (begin
    (v5-prng-step! xb P u hi h11 one11 M)       ;; x1
    (v5-zero! sB 6)
    (v5-copy! xb sB 4)
    (limb-set! one 0 1)
    (limb-add sB one sB 6 1)                    ;; S = x1 + 1
    (v5-prng-step! xb P u hi h11 one11 M)       ;; x2
    (v5-zero! hB 6)
    (v5-copy! xb hB 4)
    (loop ((j 0))
      (if (>= j 74) 0 (begin (v5-halve! hB 6) (recur (+ j 1)))))
    ;; hB now holds dy = x2>>74 (< 2^46 → up to TWO base-1e9 limbs).
    ;; Keep the FULL buffer — reading limb 0 alone truncates dy mod 1e9.
    (let ((dyx (+ (limb-get hB 0) (* (limb-get hB 1) 1000000000))))
      (begin
        (v5-zero! Lh 6)
        (v5-copy! (if (= (mod i 4) 0) L0
                   (if (= (mod i 4) 1) L1
                   (if (= (mod i 4) 2) L2 L3))) Lh 6)
        (if (= op 0)
            (begin
              (if (<= (v5-strip Lh 12) 0)
                  (begin (v5-zero! tE 12) (v5-copy! C127 tE 6))
                  (begin
                    (v5-tstar! hB Lh tA tB tC q r nbs s one)
                    (v5-zero! tD 12) (v5-copy! sB tD 6)
                    (v5-zero! tE 12) (v5-copy! q tE 10)
                    (limb-add tD tE tE 10 10)
                    (if (> (limb-cmp tE C127 10 10) 0)
                        (begin (v5-zero! tE 12) (v5-copy! C127 tE 6))
                        0))))
            0)
        (if (= op 1)
            (begin
              (if (<= (v5-strip Lh 12) 0)
                  (begin (v5-zero! tE 12) (limb-set! tE 0 1) 0)
                  (begin
                    (v5-tstar! hB Lh tA tB tC q r nbs s one)
                    (v5-zero! tD 12) (v5-copy! sB tD 6)
                    (if (> (limb-cmp tD q 10 10) 0)
                        (begin (limb-sub tD q tE 10 10) 0)
                        (begin (v5-zero! tE 12) (limb-set! tE 0 1) 0)))))
            0)
        (if (= op 3)
            ;; sfdx: exact ref replication (see v5-op-sfdx). Batch S =
            ;; x1+1 < 2^127 always, so no S≥C127 branch. hB holds d.
            (begin
              (v5-zero! tA 12) (v5-copy! sB tA 6)
              (limb-add tA C127 tA 10 10)
              (v5-halve! tA 10)
              (limb-sub tA sB tB 10 10)
              (v5-zero! tC 20)
              (limb-mul Lh tB tC 6 10)
              (v5-zero! tD 20)
              (limb-mul sB tA tD 6 10)
              (v5-zero! q 10) (v5-zero! r 10)
              (v5-divmod! tC tD q r nbs s one)
              (if (<= (limb-cmp q hB 10 6) 0)
                  (begin (v5-zero! tE 12) (v5-copy! sB tE 6))
                  (begin (v5-zero! tE 12) (v5-copy! C127 tE 6))))
            0)
        (if (= op 2)
            (begin
              (v5-zero! tA 20)
              (limb-mul Lh sB tA 6 6)
              (limb-set! one 0 1)
              (limb-add tA one tA 10 1)
              (v5-u64! tC (+ dyx 1))
              (v5-zero! tB 12)
              (limb-add tB Lh tB 10 10)
              (v5-zero! tD 20)
              (limb-mul tC sB tD 6 6)
              (v5-zero! tF 12)
              (v5-copy! tB tF 10)
              (limb-add tF tD tF 10 10)
              (v5-zero! tE 12)
              (v5-copy! tA tE 10)
              (limb-add tE tF tE 10 10)
              (limb-set! one 0 1)
              (limb-sub tE one tE 10 1)
              (v5-zero! q 10) (v5-zero! r 10)
              (v5-divmod! tE tF q r nbs s one)
              (v5-zero! tE 12)
              (v5-copy! q tE 10)
              (if (> (limb-cmp tE sB 10 10) 0)
                  (begin (v5-zero! tE 12) (v5-copy! sB tE 6))
                  0))
            0)
        (v5-modm! sumb tE 10 hi2 h11b t9 one11 M)
        (if (< i 8)
            (loop ((jj 0))
              (if (>= jj 7) 0
                  (begin
                    (limb-set! rvec (+ (* i 7) jj) (limb-get tE jj))
                    (recur (+ jj 1)))))
            0)
        (+ i 1)))))

(define (v5b-fee-step F0 F1 F2 F3 F4 F5 F6 xb n6 P u hi h11 one11 M
                      sumb hi2 h11b t9 out t rvec i)
  (begin
    (v5-prng-step! xb P u hi h11 one11 M)
    (v5-zero! n6 6)
    (v5-copy! xb n6 2)                          ;; amt = x mod 10^18
    (v5-fee! n6 (if (= (mod i 7) 0) F0
                 (if (= (mod i 7) 1) F1
                 (if (= (mod i 7) 2) F2
                 (if (= (mod i 7) 3) F3
                 (if (= (mod i 7) 4) F4
                 (if (= (mod i 7) 5) F5 F6)))))) out t)
    (v5-modm! sumb out 10 hi2 h11b t9 one11 M)
    (if (< i 8)
        (loop ((jj 0))
          (if (>= jj 7) 0
              (begin
                (limb-set! rvec (+ (* i 7) jj) (limb-get out jj))
                (recur (+ jj 1)))))
        0)
    (+ i 1)))

;; ── batch JSON result + entries ────────────────────────────────────────

(define (v5-slot-str rvec k tmp)
  (begin
    (v5-copy-off! rvec (* k 7) tmp 7)
    (limbs->str tmp (if (> (v5-strip tmp 7) 0) (v5-strip tmp 7) 1))))

(define (v5-json-slots j rvec k tmp)
  (if (>= k 8)
      j
      (v5-json-slots (json-set j (str-cat "s" (to-string k))
                               (v5-slot-str rvec k tmp))
                     rvec (+ k 1) tmp)))

(define (v5-json-result n sumb rvec tmp)
  (begin
    (v5-copy! sumb tmp 4)
    (v5-json-slots
     (json-set (json-set "{}" "n" (to-string n)) "sum"
               (limbs->str tmp (if (> (v5-strip tmp 4) 0) (v5-strip tmp 4) 1)))
     rvec 0 tmp)))

;; shared init helper: seed xb from string, M/one11 bufs
(define (v5-batch-init seedstr)
  ;; returns xb buf; caller builds M/one11 itself (needs distinct bufs)
  (let ((xb (buf-alloc 16)) (S0 (str->limbs seedstr)))
    (begin (v5-zero! xb 4) (v5-copy! (car S0) xb (car (cdr S0))) xb)))

(define (v5-mk-m)
  (let ((m (buf-alloc 48)) (A (str->limbs "999999999999999999999999999999999989")))
    (begin (v5-zero! m 12) (v5-copy! (car A) m (car (cdr A))) m)))

(define (v5-mk-one11)
  (let ((b (buf-alloc 8))) (begin (limb-set! b 0 11) b)))

(define (v5b-isqrt seedstr n)
  (let ((xb (v5-batch-init seedstr)) (M (v5-mk-m)) (one11 (v5-mk-one11))
        (n6 (buf-alloc 48)) (P (buf-alloc 48)) (u (buf-alloc 48))
        (hi (buf-alloc 48)) (h11 (buf-alloc 48)) (sumb (buf-alloc 48))
        (hi2 (buf-alloc 48)) (h11b (buf-alloc 48)) (t9 (buf-alloc 48))
        (s (buf-alloc 160)) (rem (buf-alloc 48)) (root (buf-alloc 48))
        (t0 (buf-alloc 48)) (t1 (buf-alloc 48)) (nbs (buf-alloc 48))
        (rvec (buf-alloc 256)) (tmp (buf-alloc 48)))
    (begin
      (v5-zero! sumb 12)
      (v5-zero! rvec 64)
      (loop ((i 0))
        (if (>= i n)
            (v5-json-result n sumb rvec tmp)
            (recur (v5b-isqrt-step xb n6 P u hi h11 one11 M sumb hi2 h11b
                                   t9 s rem root t0 t1 nbs rvec i)))))))

(define (v5b-spow seedstr n)
  (let ((xb (v5-batch-init seedstr)) (M (v5-mk-m)) (one11 (v5-mk-one11))
        (n6 (buf-alloc 48)) (P (buf-alloc 48)) (u (buf-alloc 48))
        (hi (buf-alloc 48)) (h11 (buf-alloc 48)) (sumb (buf-alloc 48))
        (hi2 (buf-alloc 48)) (h11b (buf-alloc 48)) (t9 (buf-alloc 48))
        (x6 (buf-alloc 48)) (dv (buf-alloc 48)) (acc (buf-alloc 88))
        (R (buf-alloc 88)) (P40 (buf-alloc 176)) (t24 (buf-alloc 120))
        (N (buf-alloc 64)) (nbs2 (buf-alloc 64)) (s2 (buf-alloc 256))
        (rem (buf-alloc 48)) (root (buf-alloc 48)) (t0 (buf-alloc 48))
        (t1 (buf-alloc 48)) (D (buf-alloc 88)) (Bpos (buf-alloc 88))
        (Bneg (buf-alloc 88))
        (BpA (str->limbs "1000100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"))
        (BnA (str->limbs "10000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000")) (BnS (buf-alloc 64))
        (q (buf-alloc 64)) (r (buf-alloc 64)) (nbs (buf-alloc 64))
        (s (buf-alloc 400)) (one (buf-alloc 8)) (rvec (buf-alloc 256))
        (tmp (buf-alloc 48)))
    (begin
      (v5-zero! sumb 12)
      (v5-zero! rvec 64)
      (v5-zero! D 22) (limb-set! D 12 1)
      (v5-zero! Bpos 22) (v5-copy! (car BpA) Bpos 13)
      (begin
        (v5-zero! Bneg 22)
        (v5-sdiv! (car BnA) 10001 BnS 13)
        (v5-copy! BnS Bneg 13))
      (loop ((i 0))
        (if (>= i n)
            (v5-json-result n sumb rvec tmp)
            (recur (v5b-spow-step xb x6 dv P u hi h11 one11 M sumb hi2
                                  h11b t9 acc R P40 t24 N nbs2 s2 rem root
                                  t0 t1 D Bpos Bneg q r nbs s one rvec i)))))))

(define (v5b-seg seedstr n up op)
  (let ((xb (v5-batch-init seedstr)) (M (v5-mk-m)) (one11 (v5-mk-one11))
        (n6 (buf-alloc 48)) (P (buf-alloc 48)) (u (buf-alloc 48))
        (hi (buf-alloc 48)) (h11 (buf-alloc 48)) (sumb (buf-alloc 48))
        (hi2 (buf-alloc 48)) (h11b (buf-alloc 48)) (t9 (buf-alloc 48))
        (d (buf-alloc 32)) (PQP (buf-alloc 64)) (PQQ (buf-alloc 64))
        (out (buf-alloc 64)) (qr (buf-alloc 64)) (rr (buf-alloc 64))
        (nbs (buf-alloc 64)) (s (buf-alloc 400)) (one (buf-alloc 8))
        (sB (buf-alloc 48)) (s2B (buf-alloc 48)) (hB (buf-alloc 48))
        (Lh (buf-alloc 48)) (rvec (buf-alloc 256)) (tmp (buf-alloc 48))
        (L0 (v5-str6 "0")) (L1 (v5-str6 "1"))
        (L2 (v5-str6 "1000000000000000000"))
        (L3 (v5-str6 "500000000000000000")))
    (begin
      (v5-zero! sumb 12)
      (v5-zero! rvec 64)
      (loop ((i 0))
        (if (>= i n)
            (v5-json-result n sumb rvec tmp)
            (recur (v5b-seg-step up op L0 L1 L2 L3 xb n6 P u hi h11 one11
                                  M sumb hi2 h11b t9 d PQP PQQ out qr rr
                                  nbs s one sB s2B hB Lh rvec i)))))))

(define (v5b-sfor seedstr n op)
  (let ((xb (v5-batch-init seedstr)) (M (v5-mk-m)) (one11 (v5-mk-one11))
        (n6 (buf-alloc 48)) (P (buf-alloc 48)) (u (buf-alloc 48))
        (hi (buf-alloc 48)) (h11 (buf-alloc 48)) (sumb (buf-alloc 48))
        (hi2 (buf-alloc 48)) (h11b (buf-alloc 48)) (t9 (buf-alloc 48))
        (tA (buf-alloc 80)) (tB (buf-alloc 80)) (tC (buf-alloc 88))
        (tD (buf-alloc 80)) (tE (buf-alloc 80)) (tF (buf-alloc 80))
        (one (buf-alloc 8)) (q (buf-alloc 64)) (r (buf-alloc 64))
        (nbs (buf-alloc 64)) (s (buf-alloc 400)) (sB (buf-alloc 48))
        (hB (buf-alloc 48)) (Lh (buf-alloc 48)) (rvec (buf-alloc 256))
        (tmp (buf-alloc 48))
        (C127 (v5-str6 "170141183460469231731687303715884105728"))
        (L0 (v5-str6 "0")) (L1 (v5-str6 "1"))
        (L2 (v5-str6 "1000000000000000000"))
        (L3 (v5-str6 "500000000000000000")))
    (begin
      (v5-zero! sumb 12)
      (v5-zero! rvec 64)
      (loop ((i 0))
        (if (>= i n)
            (v5-json-result n sumb rvec tmp)
            (recur (v5b-sfor-step op L0 L1 L2 L3 xb n6 P u hi h11 one11 M
                                  sumb hi2 h11b t9 tA tB tC tD tE tF one
                                  q r nbs s sB hB Lh C127 rvec i)))))))

(define (v5b-fee seedstr n)
  (let ((xb (v5-batch-init seedstr)) (M (v5-mk-m)) (one11 (v5-mk-one11))
        (n6 (buf-alloc 48)) (P (buf-alloc 48)) (u (buf-alloc 48))
        (hi (buf-alloc 48)) (h11 (buf-alloc 48)) (sumb (buf-alloc 48))
        (hi2 (buf-alloc 48)) (h11b (buf-alloc 48)) (t9 (buf-alloc 48))
        (out (buf-alloc 64)) (t (buf-alloc 64)) (rvec (buf-alloc 256))
        (tmp (buf-alloc 48))
        (F0 (v5-str6 "0")) (F1 (v5-str6 "1")) (F2 (v5-str6 "100"))
        (F3 (v5-str6 "400")) (F4 (v5-str6 "2000")) (F5 (v5-str6 "9999"))
        (F6 (v5-str6 "10000")))
    (begin
      (v5-zero! sumb 12)
      (v5-zero! rvec 64)
      (loop ((i 0))
        (if (>= i n)
            (v5-json-result n sumb rvec tmp)
            (recur (v5b-fee-step F0 F1 F2 F3 F4 F5 F6 xb n6 P u hi h11
                                 one11 M sumb hi2 h11b t9 out t rvec i)))))))

;; ── _run dispatch ──────────────────────────────────────────────────────

(define (v5-run)
  (let ((inp (near/input)))
    (let ((op (json-get-str "op" inp)))
      (cond
        ((str= op "isqrt")
         (near/return (v5-op-isqrt-newton (json-get-str "n" inp))))
        ((str= op "isqrt_dd")
         (near/return (v5-op-isqrt-dd (json-get-str "n" inp))))
        ((str= op "spow")
         (near/return (v5-op-spow (str->num (json-get-str "p" inp)))))
        ((str= op "dy")
         (near/return (v5-op-dy (json-get-str "L" inp)
                                (json-get-str "S" inp)
                                (json-get-str "S2" inp))))
        ((str= op "dx")
         (near/return (v5-op-dx (json-get-str "L" inp)
                                (json-get-str "S" inp)
                                (json-get-str "S2" inp))))
        ((str= op "dyi")
         (near/return (v5-op-dy (json-get-str "L" inp)
                                (json-get-str "S2" inp)
                                (json-get-str "S" inp))))
        ((str= op "dxo")
         (near/return (v5-op-dx (json-get-str "L" inp)
                                (json-get-str "S2" inp)
                                (json-get-str "S" inp))))
        ((str= op "sfdy")
         (near/return (v5-op-sfdy (json-get-str "L" inp)
                                  (json-get-str "S" inp)
                                  (json-get-str "dy" inp))))
        ((str= op "sfdx")
         (near/return (v5-op-sfdx (json-get-str "L" inp)
                                  (json-get-str "S" inp)
                                  (json-get-str "dx" inp))))
        ((str= op "sfdyi")
         (near/return (v5-op-sfdyi (json-get-str "L" inp)
                                   (json-get-str "S" inp)
                                   (json-get-str "dy" inp))))
        ((str= op "sfdxo")
         (near/return (v5-op-sfdxo (json-get-str "L" inp)
                                   (json-get-str "S" inp)
                                   (json-get-str "dx" inp))))
        ((str= op "fee")
         (near/return (v5-op-fee (json-get-str "amt" inp)
                                 (json-get-str "bps" inp))))
        (else (v5-run-batch inp op))))))

(define (v5-run-batch inp op)
  (cond
    ((str= op "isqrt_batch")
     (near/return (v5b-isqrt (json-get-str "seed" inp)
                             (str->num (json-get-str "n" inp)))))
    ((str= op "spow_batch")
     (near/return (v5b-spow (json-get-str "seed" inp)
                            (str->num (json-get-str "n" inp)))))
    ((str= op "dy_batch")
     (near/return (v5b-seg (json-get-str "seed" inp)
                           (str->num (json-get-str "n" inp)) 1 0)))
    ((str= op "dx_batch")
     (near/return (v5b-seg (json-get-str "seed" inp)
                           (str->num (json-get-str "n" inp)) 1 1)))
    ((str= op "dyi_batch")
     (near/return (v5b-seg (json-get-str "seed" inp)
                           (str->num (json-get-str "n" inp)) 0 0)))
    ((str= op "dxo_batch")
     (near/return (v5b-seg (json-get-str "seed" inp)
                           (str->num (json-get-str "n" inp)) 0 1)))
    ((str= op "sfdy_batch")
     (near/return (v5b-sfor (json-get-str "seed" inp)
                            (str->num (json-get-str "n" inp)) 0)))
    ((str= op "sfdx_batch")
     (near/return (v5b-sfor (json-get-str "seed" inp)
                            (str->num (json-get-str "n" inp)) 3)))
    ((str= op "sfdyi_batch")
     (near/return (v5b-sfor (json-get-str "seed" inp)
                            (str->num (json-get-str "n" inp)) 1)))
    ((str= op "sfdxo_batch")
     (near/return (v5b-sfor (json-get-str "seed" inp)
                            (str->num (json-get-str "n" inp)) 2)))
    ((str= op "fee_batch")
     (near/return (v5b-fee (json-get-str "seed" inp)
                           (str->num (json-get-str "n" inp)))))
    (else (near/return "{\"err\":\"bad-op\"}"))))

(export "_run" v5-run)
