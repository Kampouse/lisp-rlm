;;; Test let and loop constructs

;;; --- basic let ---
(let ((x 10) (y 20)) (+ x y))

;;; --- let with shadowing ---
(define x 100)
(let ((x 1)) x)

;;; --- nested let ---
(let ((a 5))
  (let ((b 10))
    (+ a b)))

;;; --- let with expression bindings ---
(let ((x (+ 1 2)) (y (* 3 4))) (+ x y))

;;; --- named let (loop) ---
(let countdown ((n 5))
  (if (= n 0) 0
    (countdown (- n 1))))

;;; --- named let accumulating ---
(let loop ((i 1) (acc 0))
  (if (> i 10) acc
    (loop (+ i 1) (+ acc i))))

;;; --- basic loop/recur ---
(loop ((i 0))
  (if (= i 10) i
    (recur (+ i 1))))

;;; --- loop with accumulator ---
(loop ((n 10) (acc 1))
  (if (= n 0) acc
    (recur (- n 1) (* acc n))))

;;; --- loop building list ---
(loop ((i 5) (acc ()))
  (if (= i 0) acc
    (recur (- i 1) (cons i acc))))

;;; --- let with lambda ---
(define adder
  (let ((n 10))
    (lambda (x) (+ x n))))
(adder 5)

;;; --- do loop ---
(do ((i 0 (+ i 1)))
    ((= i 5) i)
  i)

;;; --- dotimes ---
(dotimes (i 5) i)

;;; --- while loop with mutation ---
(define counter 0)
(define result '())
(while (< counter 4)
  (set! counter (+ counter 1))
  (set! result (cons counter result)))
result

;;; --- map over list with let ---
(map (lambda (x) (let ((doubled (* x 2))) doubled)) (list 1 2 3 4 5))

;;; --- filter with let ---
(filter (lambda (x) (let ((is-even (= (% x 2) 0))) is-even)) (list 1 2 3 4 5 6))

;;; --- reduce with let ---
(reduce (lambda (acc x) (let ((sum (+ acc x))) sum)) 0 (list 1 2 3 4 5))
