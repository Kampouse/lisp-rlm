;; T2 — peephole/jump remap: typed arithmetic mixed into branch + loop bodies
;; (the exact shape that exposed the index_map off-by-one)
(define (t2 x)
  (let ((f 1.5) (i 2))
    (if (> x 0)
        (set! f (+ f 0.5))
        (set! i (+ i 1)))
    (while (< i 5)
      (set! i (+ i 1))
      (if (> i 3) (set! f (+ f 1.0)) 0))
    (+ f i)))
(println (t2 1))   ; f: 2.0→4.0, i=5 → 9.0
(println (t2 0))   ; i=3 start, f 1.5→3.5 → 8.5
