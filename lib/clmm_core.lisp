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
