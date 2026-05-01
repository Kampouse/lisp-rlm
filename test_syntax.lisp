(define r1 42)
(define r2 -7)
(define r3 3.14)
(define r4 true)
(define r5 false)
(define r6 "hello")
(define r7 nil)
(define r8 'foo)
(define r9 '(1 2 3))

(define a1 (+ 1 2))
(define a2 (- 10 3))
(define a3 (* 4 5))
(define a4 (/ 20 4))
(define a5 (% 7 3))
(define a6 (+ 1 2 3 4 5))
(define a7 (* 2 3 4))

(define c1 (< 1 2))
(define c2 (<= 2 2))
(define c3 (> 3 2))
(define c4 (>= 3 3))
(define c5 (= 5 5))
(define c6 (!= 1 2))

(define l1 (and true true))
(define l2 (or false true))
(define l3 (not false))

(define x 10)
(set! x 20)

(define square (lambda (n) (* n n)))
(define add1 (lambda (x) (+ x 1)))
(define multi-body (lambda (x) (+ x 1) (+ x 2)))
(define my-list (lambda args args))

(define let1 (let ((a 1) (b 2)) (+ a b)))
(define let2 (let* ((a 1) (b (+ a 1))) (+ a b)))
(define let3 (let ((x 10))
  (let ((y 20))
    (+ x y))))

(define if1 (if true 1 2))
(define if2 (if false 1 2))
(define if3 (if (> 3 2) "yes" "no"))

(define cond1
  (cond
    ((= 1 2) "nope")
    ((= 1 1) "yes")
    (else "default")))

(define begin1 (begin 1 2 3))
(define progn1 (progn 4 5 6))

(define loop1
  (let loop ((i 0) (acc 0))
    (if (= i 5)
      acc
      (loop (+ i 1) (+ acc i)))))

(define recur1
  (let loop ((n 10) (acc 0))
    (if (= n 0)
      acc
      (recur (- n 1) (+ acc n)))))

(define make-adder
  (lambda (n)
    (lambda (x) (+ n x))))
(define add5 (make-adder 5))

(define lst1 (list 1 2 3))
(define lst2 (car lst1))
(define lst3 (cdr lst1))
(define lst4 (cons 0 lst1))
(define lst5 (null? nil))
(define lst6 (pair? lst1))
(define lst7 (length lst1))

(define mapped (map (lambda (x) (* x x)) (list 1 2 3)))
(define filtered (filter (lambda (x) (> x 2)) (list 1 2 3 4)))
(define reduced (reduce (lambda (acc x) (+ acc x)) 0 (list 1 2 3 4 5)))

(define s1 (string-append "hello" " " "world"))
(define s2 (string-length "abc"))
(define s3 (string=? "foo" "foo"))

(define t1 (number? 42))
(define t2 (string? "hi"))
(define t3 (boolean? true))
(define t4 (nil? nil))
(define t5 (list? (list 1 2)))
(define t6 (lambda? square))

(define do1
  (do ((i 0 (+ i 1))
       (acc 0 (+ acc i)))
      ((= i 5) acc)))

(define when1 (when true 42))
(define unless1 (unless false 99))

(define andor1 (and 1 2 3))
(define andor2 (or false false 42))

(define apply1 (apply + (list 1 2 3)))

(define let-in-loop
  (let loop ((i 0))
    (if (= i 3)
      i
      (let ((x (* i 10)))
        (loop (+ i 1))))))

(display "=== LITERALS ===") (newline)
(display r1) (display " ") (display r2) (display " ") (display r3) (newline)
(display r4) (display " ") (display r5) (display " ") (display r6) (newline)
(display r7) (display " ") (display r8) (display " ") (display r9) (newline)

(display "=== ARITHMETIC ===") (newline)
(display a1) (display " ") (display a2) (display " ") (display a3) (display " ")
(display a4) (display " ") (display a5) (display " ") (display a6) (display " ")
(display a7) (newline)

(display "=== COMPARISON ===") (newline)
(display c1) (display " ") (display c2) (display " ") (display c3) (display " ")
(display c4) (display " ") (display c5) (display " ") (display c6) (newline)

(display "=== LOGIC ===") (newline)
(display l1) (display " ") (display l2) (display " ") (display l3) (newline)

(display "=== DEFINE/SET ===") (newline)
(display x) (newline)

(display "=== LAMBDA ===") (newline)
(display (square 5)) (display " ") (display (add1 10)) (display " ")
(display (multi-body 5)) (display " ") (display (my-list 1 2 3)) (newline)

(display "=== LET ===") (newline)
(display let1) (display " ") (display let2) (display " ") (display let3) (newline)

(display "=== IF/COND ===") (newline)
(display if1) (display " ") (display if2) (display " ") (display if3) (newline)
(display cond1) (newline)

(display "=== BEGIN/PROGN ===") (newline)
(display begin1) (display " ") (display progn1) (newline)

(display "=== LOOP (named let) ===") (newline)
(display loop1) (newline)

(display "=== RECUR ===") (newline)
(display recur1) (newline)

(display "=== CLOSURES ===") (newline)
(display (add5 3)) (newline)

(display "=== LIST OPS ===") (newline)
(display lst1) (display " car=") (display lst2) (display " cdr=") (display lst3)
(display " cons=") (display lst4) (newline)
(display "null?=") (display lst5) (display " pair?=") (display lst6)
(display " length=") (display lst7) (newline)

(display "=== MAP/FILTER/REDUCE ===") (newline)
(display mapped) (newline)
(display filtered) (newline)
(display reduced) (newline)

(display "=== STRING OPS ===") (newline)
(display s1) (newline)
(display s2) (display " ") (display s3) (newline)

(display "=== TYPE PREDICATES ===") (newline)
(display t1) (display " ") (display t2) (display " ") (display t3) (display " ")
(display t4) (display " ") (display t5) (display " ") (display t6) (newline)

(display "=== DO ===") (newline)
(display do1) (newline)

(display "=== WHEN/UNLESS ===") (newline)
(display when1) (display " ") (display unless1) (newline)

(display "=== AND/OR values ===") (newline)
(display andor1) (display " ") (display andor2) (newline)

(display "=== APPLY ===") (newline)
(display apply1) (newline)

(display "=== LET IN LOOP ===") (newline)
(display let-in-loop) (newline)

(display "=== DONE ===") (newline)
