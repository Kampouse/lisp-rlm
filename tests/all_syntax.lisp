;;; ============================================================
;;; COMPREHENSIVE SYNTAX COVERAGE FILE
;;; Tests every construct the lisp-rlm compiler should handle.
;;; Run with: cargo run --bin lisp-run -- --coverage tests/all_syntax.lisp
;;; ============================================================

;;; --- 1. LITERALS ---
42
-7
3.14
"hello world"
true
false
nil
()

;;; --- 2. QUOTE ---
(quote foo)
'bar
'(1 2 3)

;;; --- 3. ARITHMETIC ---
(+ 1 2 3)
(- 10 3 2)
(* 6 7)
(/ 10 3)
(% 10 3)
(+ (* 2 3) (- 10 4))

;;; --- 4. COMPARISON ---
(= 42 42)
(< 1 2)
(> 2 1)
(<= 2 2)
(>= 3 2)

;;; --- 5. LOGIC ---
(and 1 2 3)
(and 1 false 3)
(or false nil 42)
(not false)
(not 42)

;;; --- 6. IF ---
(if true 1 2)
(if false 1 2)
(if false 1)

;;; --- 7. COND ---
(cond ((< 1 0) "a") ((> 2 1) "b") (else "c"))

;;; --- 8. WHEN/UNLESS ---
(when true 42)
(when false 42)
(unless false 99)
(unless true 99)

;;; --- 9. DEFINE ---
(define x 42)
x
(define (add a b) (+ a b))
(add 3 4)

;;; --- 10. LET ---
(let ((a 10) (b 20)) (+ a b))
(let* ((a 10) (b (+ a 5))) b)

;;; --- 11. SET! ---
(define counter 0)
(set! counter 42)
counter

;;; --- 12. LAMBDA ---
((lambda (x) (+ x 1)) 5)
((lambda (a b &rest rest) (len rest)) 1 2 3 4)

;;; --- 13. BEGIN/PROGN/DO ---
(begin 1 2 3)
(progn 1 2 3)
(do 1 2 3)

;;; --- 14. LOOP/RECUR ---
(loop ((i 0)) (if (>= i 5) i (recur (+ i 1))))
(loop ((i 0) (sum 0)) (if (> i 10) sum (recur (+ i 1) (+ sum i))))

;;; --- 15. NAMED LET ---
(let countdown ((n 5)) (if (= n 0) 0 (countdown (- n 1))))

;;; --- 16. QUASIQUOTE ---
(let ((x 42)) (quasiquote (+ (unquote x) 1)))
(let ((xs (list 1 2 3))) (quasiquote (0 (unquote-splicing xs) 4)))

;;; --- 17. COLLECTIONS ---
(list 1 2 3)
(car (list 1 2 3))
(cdr (list 1 2 3))
(cons 0 (list 1 2))
(len (list 1 2 3))
(append (list 1 2) (list 3 4))
(nth 1 (list 10 20 30))
(reverse (list 1 2 3))
(sort (list 3 1 2))
(range 0 5)

;;; --- 18. HIGHER-ORDER ---
(map (lambda (x) (* x 2)) (list 1 2 3))
(filter (lambda (x) (> x 2)) (list 1 2 3))
(reduce + 0 (list 1 2 3))

;;; --- 19. DICT ---
(define d (dict "a" 1 "b" 2))
(dict/get d "a")
(dict/has? d "b")
(dict/set d "c" 3)

;;; --- 20. STRINGS ---
(str-concat "hello" " " "world")
(str-contains "hello" "ell")
(str-length "hello")
(str-split "a,b,c" ",")
(to-string 42)

;;; --- 21. TYPE INTROSPECTION ---
(type? 42)
(type? "hello")
(type? true)
(type? nil)

;;; --- 22. MATCH ---
(match 42 (_ "matched"))
(match 42 (1 "one") (42 "found") (_ "other"))
(match "hello" ("world" 1) ("hello" 2) (_ 3))
(match (list 1 2) ((list ?a ?b) (+ a b)) (_ 0))

;;; --- 23. TRY/CATCH ---
(try (+ 1 2) (catch e 0))
(try (/ 1 0) (catch e (str-concat "caught: " e)))

;;; --- 24. MACROS ---
(defmacro swap (a b) (list (quote list) b a))
(swap 1 2)

;;; --- 25. DEFTYPE ---
(deftype Color Red Green Blue)
(Red)
(deftype Shape (Circle 1) (Rect 2))
(Circle 5.0)

;;; --- 26. CONTRACTS ---
(define checked-add (contract ((a :int) (b :int) -> :int) (+ a b)))
(checked-add 3 4)

;;; --- 27. COMPLEX: FIBONACCI ---
(define (fib n) (if (<= n 1) n (+ (fib (- n 1)) (fib (- n 2)))))
(fib 10)

;;; --- 28. COMPLEX: MUTUAL RECURSION ---
(define my-even? nil)
(define my-odd? nil)
(set! my-even? (lambda (n) (if (= n 0) true (my-odd? (- n 1)))))
(set! my-odd? (lambda (n) (if (= n 0) false (my-even? (- n 1)))))
(my-even? 10)

;;; --- 29. CLOSURE CHAIN ---
(define (compose f g) (lambda (x) (f (g x))))
(define add1 (lambda (x) (+ x 1)))
(define double (lambda (x) (* x 2)))
((compose add1 double) 5)

;;; --- 30. LOOP WITH LIST PROCESSING ---
(define (sum-list lst)
  (loop ((remaining lst) (acc 0))
    (if (nil? remaining) acc
      (recur (cdr remaining) (+ acc (car remaining))))))
(sum-list (list 1 2 3 4 5))
