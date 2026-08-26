;; T17 — desugar equivalence: while vs dotimes vs named recursion vs
;; loop/recur must produce IDENTICAL results for the same recurrence
;;
;; Ground rules: (2) sum of squares 1..7 = 140, 1..12 = 650 (python3);
;; fib(15) = 610. Any divergence between the four forms is a desugaring
;; bug, not a math bug.

(define (sq n) (* n n))

(define (sum-sq-while n)
  (let ((i 1) (acc 0))
    (while (<= i n)
      (set! acc (+ acc (sq i)))
      (set! i (+ i 1)))
    acc))

(define (sum-sq-dotimes n)
  (let ((acc 0))
    (dotimes (k n)
      (set! acc (+ acc (sq (+ k 1)))))
    acc))

(define (sum-sq-rec n)
  (if (= n 0) 0 (+ (sq n) (sum-sq-rec (- n 1)))))

(define (sum-sq-loop n)
  (loop ((i 1) (acc 0))
    (if (> i n) acc (recur (+ i 1) (+ acc (sq i))))))

(println (sum-sq-while 7))    ; 140
(println (sum-sq-dotimes 7))  ; 140
(println (sum-sq-rec 7))      ; 140
(println (sum-sq-loop 7))     ; 140
(println (sum-sq-while 12))   ; 650
(println (sum-sq-dotimes 12)) ; 650
(println (sum-sq-rec 12))     ; 650
(println (sum-sq-loop 12))    ; 650

(define (fib-rec n) (if (< n 2) n (+ (fib-rec (- n 1)) (fib-rec (- n 2)))))
(define (fib-loop n)
  (loop ((i 2) (a 0) (b 1))
    (if (> i n) b (recur (+ i 1) b (+ a b)))))
(define (fib-while n)
  (let ((i 2) (a 0) (b 1))
    (while (<= i n)
      (let ((next (+ a b)))
        (set! a b)
        (set! b next)
        (set! i (+ i 1))))
    b))
(println (fib-rec 15))   ; 610
(println (fib-loop 15))  ; 610
(println (fib-while 15)) ; 610
(println (fib-rec 1))    ; 1
(println (fib-loop 1))   ; 1  — loop body never runs, returns b=1
(println (fib-while 1))  ; 1
