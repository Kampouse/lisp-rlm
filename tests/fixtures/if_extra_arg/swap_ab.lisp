;; swap_ab.lisp — semantic A/B skeleton for the if-arity cap (the CLEAN
;; state; the corrupted twin lives at
;; tests/compiler-torture/t22-if-extra-arg-err.lisp and must NOT compile).
;; Shape = the CLMM v5 pool dispatch: near methods take NO lisp params —
;; they read (near/input) + json-get-str. The then-arm carries a
;; multi-binding let* (second binding references the first — sequential
;; semantics). Runtime assertions live in
;; tests/test_if_extra_arg_regression.rs: down input must return the DOWN
;; arm's marker only (pre-guard corruption made the up constant win).
(define (arm)
  (let ((v (json-get-str "v" (near/input))))
    (if (str= (str-slice v 0 1) "-")
        (let* ((a (str-cat v "!"))
               (b (str-cat a "?")))
          (str-cat "D" b))
        (str-cat "U" v))))
(export "arm" arm)
