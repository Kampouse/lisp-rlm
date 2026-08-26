;; T11 — recursion depth: non-tail self-recursion, mutual recursion, budget
;;
;; Ground rules: (2) hand-computed constants only. Task assumption "300 →
;; max_call_depth hard error" was PROBED and is FALSE for named functions:
;; direct self-recursion compiles to iterative CallSelf frames (no
;; run_compiled_lambda crossing), so call_depth never increments and the
;; 1M-op execution budget is the only ceiling. sum-to-n(10000) = 50005000
;; runs clean. Documented as actual semantics (see GAPS.md, torture r2).
;; Mutual recursion across top-level defines (LoadGlobal/CallDynamic) works
;; after the lisp-run forward-reference fix (commit 4196858) — it used to be
;; a compile error. Unlike self-recursion, every mutual hop crosses
;; run_compiled_lambda, so mutual recursion IS capped by max_call_depth=256:
;; probed boundary is depth 254 OK / 255 "call depth exceeded". TRUE
;; dynamic-dispatch recursion (closure called through a value) is capped the
;; same way — see t11b for the error leg.

(define (sum-to n) (if (= n 0) 0 (+ n (sum-to (- n 1)))))  ; non-tail
(println (sum-to 10))      ; 55
(println (sum-to 50))      ; 1275
(println (sum-to 200))     ; 20100
(println (sum-to 250))     ; 31375
(println (sum-to 255))     ; 32640
(println (sum-to 300))     ; 45150 — NO depth error: CallSelf is iterative
(println (sum-to 1000))    ; 500500
(println (sum-to 10000))   ; 50005000

(define (sum-acc n acc) (if (= n 0) acc (sum-acc (- n 1) (+ acc n))))  ; tail
(println (sum-acc 10000 0)) ; 50005000

;; mutual recursion across two top-level defines (forward reference)
(define (my-even? n) (if (= n 0) true (my-odd? (- n 1))))
(define (my-odd? n) (if (= n 0) false (my-even? (- n 1))))
(println (my-even? 200))   ; true
(println (my-odd? 201))    ; true  (201 crossings + body < 256)
