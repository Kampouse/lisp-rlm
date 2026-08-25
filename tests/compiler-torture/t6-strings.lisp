;; T6 — string builtins in loop context (old str-cat collision class)
(define (tag i) (str-cat "key-" i))
(define (t6 n)
  (let ((out ""))
    (dotimes (i n)
      (set! out (str-cat out (tag i))))
    out))
(println (t6 4))   ; key-0key-1key-2key-3
(println (str-cat "x" 42))  ; x42
