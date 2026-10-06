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
                (str-cat (car (cdr s4)) "/" (car s4))))))))))

(define (pool-state)
  (str-cat "s0:" (slot-get 0) " s1:" (slot-get 1) " s2:" (slot-get 2)
           " s3:" (slot-get 3) " s4:" (slot-get 4) " F:" (fee-get)))
