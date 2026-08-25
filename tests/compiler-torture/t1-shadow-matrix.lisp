;; T1 — slot allocator: shadowing in every direction
;; let-shadow + loop-shadow-let + loop-on-loop, sequential same-name
(define (t1 n)
  (let ((a 0) (b 0))
    (dotimes (i n)
      (let ((a i))
        (dotimes (a n)
          (set! b (+ b a)))))
    (+ a b)))
(println (t1 3))   ; each outer adds (0+1+2)=3, 3 outers → b=9, a=0 → 9
(println (t1 4))   ; 6 per outer × 4 → 24
