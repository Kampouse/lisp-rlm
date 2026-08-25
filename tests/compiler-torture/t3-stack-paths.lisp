;; T3 — stack balance: loops in non-tail position before recursion,
;; every branch returns a value, nesting after loop exit
(define (t3 n)
  (if (= n 0)
      42
      (begin
        (dotimes (k n) 0)
        (t3 (- n 1)))))
(println (t3 5))   ; 42
(println (t3 0))   ; 42
(define (t3b n acc)
  (if (= n 0)
      acc
      (begin
        (while (> n 2) (set! n (- n 1)))
        (t3b (- n 1) (+ acc n)))))
(println (t3b 9 0))  ; n: 9→2 via while, acc+=2, then n=1, acc+=1 → 3
