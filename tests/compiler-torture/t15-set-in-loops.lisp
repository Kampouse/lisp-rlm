;; T15 — set! inside while/dotimes bodies: outer-scope mutation from inner
;; scopes, accumulator read in the loop CONDITION, shadowed set! targets
;;
;; Ground rules: (2) constants verified with python3 (shadowing modeled with
;; distinct variables — Python has no let-shadowing); (5) the W2 stale-env
;; class: set! from inside a loop body must write the LIVE outer binding,
;; not a creation-time snapshot.

;; 1. accumulator mutated from while body, then used in the next condition
(define (fill-until limit)
  (let ((total 0) (spins 0))
    (while (< total limit)
      (set! total (+ total 3))
      (set! spins (+ spins 1)))
    (+ (* 100 total) spins)))
(println (fill-until 20)) ; total: 3,6,...,21 (first ≥20); spins 7 → 2107
(println (fill-until 21)) ; exactly 21 → 7 spins → 2107
(println (fill-until 0))  ; loop never runs → 0

;; 2. set! of an OUTER binding from inside dotimes, with a shadowing inner
;;    let — the inner set! must hit the INNER binding only (Lisp scoping),
;;    the outer stays untouched
(define (shadowed-bump n)
  (let ((a 0))
    (dotimes (i n)
      (let ((a (+ i 1)))
        (set! a (* a 2))))     ; mutates the INNER a
      a))                      ; outer a never changed
(println (shadowed-bump 5)) ; 0 — inner shadowing: outer binding intact

;; 3. while + dotimes nested, both mutating the SAME outer accumulator that
;;    the while CONDITION reads (t15b pattern)
(define (t15b n)
  (let ((total 0) (limit (* n 2)) (i 0))
    (while (< total limit)
      (dotimes (j 3)
        (set! total (+ total 1)))
      (set! i (+ i 1)))
    (+ total (* 100 i))))
(println (t15b 2)) ; total crosses limit=4 during 2nd dotimes → 6 total, 2 spins → 206
(println (t15b 3)) ; limit=6 → 6 total, 2 spins → 206

;; 4. mixed increment per iteration driven by even? (W2 classic shape)
(define (t15 n)
  (let ((acc 0) (i 0))
    (while (< i n)
      (let ((bump (if (even? i) 1 2)))
        (set! acc (+ acc bump))
        (set! acc (+ acc 1))
        (set! i (+ i 1))))
    acc))
(println (t15 6))  ; i:0→+1+1, 1→+2+1, 2→+1+1, 3→+2+1, 4→+1+1, 5→+2+1 → 15
(println (t15 10)) ; 6 iters above + i:6..9 → 2+2+2+... : +1+1,+2+1,+1+1,+2+1 → 25
