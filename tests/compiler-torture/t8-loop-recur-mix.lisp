;; T8 — loop/recur form interplaying with new loops
(define (fact n)
  (loop ((i 1) (acc 1))
    (if (> i n) acc (recur (+ i 1) (* acc i)))))
(println (fact 6))   ; 720
(define (mixed n)
  (loop ((i 0) (acc 0))
    (if (>= i n)
        acc
        (recur (+ i 1)
               (let ((tmp 0))
                 (dotimes (j i) (set! tmp (+ tmp j)))
                 (+ acc tmp))))))
(println (mixed 5))  ; tmp sums: i=0:0, 1:0, 2:1, 3:3, 4:6 → 10
