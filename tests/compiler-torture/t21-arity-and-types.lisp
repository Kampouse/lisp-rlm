;; T21 — user-fn arity validation (round-3 fix 3, landed 2026-08-26)
;;        + arith type-coercion PINS (fix 4 tripwires, not yet fixed)
;;
;; ARITY: all user-fn call paths now hard-error on mismatch —
;; "arity mismatch: <fn> expects N args, got M" (variadic: "at least N").
;; Covers: direct calls, apply, anonymous lambdas, variadic minimums.
;; Choke point: run_compiled_lambda — vm_call_lambda, const-fold inlining,
;; apply/map/filter/reduce, and try/catch thunks all funnel through it.
;;
;; ARITH-TYPES (flipped, round-3 fix 4): bare + - * / mod < <= > >= are
;; i64/f64 ONLY — non-numeric operands hard-error. String numerics go
;; through u128/* builtins (unchanged).

;; ── arity: direct calls ──
(define (f0) 42)
(define (f2 a b) (+ a b))
(println (f0))                                ; 42 — zero-param ok
(println (f2 1 2))                            ; 3 — exact ok
(println (try (f2 1) (catch e "err-missing"))) ; err-missing
(println (try (f2 1 2 3) (catch e "err-extra"))) ; err-extra

;; ── arity: anonymous lambdas (name falls back to ops-hint) ──
(println (try ((lambda (x y) (* x y)) 5) (catch e "err-anon"))) ; err-anon

;; ── arity: apply spreads count too ──
(println (try (apply f2 (list 1)) (catch e "err-apply")))      ; err-apply
(println (apply f2 (list 1 2)))                                ; 3

;; ── arity: variadic — &rest requires AT LEAST the fixed params ──
(define (g a &rest more) (cons a more))
(println (g 1 2 3))                            ; (1 2 3) — rest packed
(println (g 1))                                ; (1) — empty rest ok
(println (try (g) (catch e "err-var-min")))    ; err-var-min — below minimum

;; ── arity: higher-order builtins pass correct counts ──
(println (map (lambda (x) (* x x)) (list 1 2 3))) ; (1 4 9)

;; ── arith/comparison types (flipped, round-3 fix 4) — i64/f64 only ──
;; Was ARITH-PIN: non-numbers coerced to 0 (or false for cmp). Now hard
;; errors: "type error: <op> expects numbers, got <a> <b>". String numerics
;; must go through u128/* (still legal, unchanged).
(println (try (+ "a" 1) (catch e "err-str")))      ; err-str (was 1)
(println (try (* (list 1 2) 10) (catch e "err-list"))) ; err-list (was 0)
(println (try (+ nil 5) (catch e "err-nil")))      ; err-nil (was 5)
(println (try (+ 1.5 "a") (catch e "err-fmix")))   ; err-fmix — float+non-num too
(println (try (< 1 "a") (catch e "err-cmp")))      ; err-cmp — comparisons typed too
(println (+ 1 2))                                  ; 3 — ints fine
(println (+ 1.5 2))                                ; 3.5 — float mixing fine
(println (< 1 2))                                  ; true
(println (<= 1.5 2))                               ; true
(println (u128/add "5" "6"))                       ; "11" — string numerics via u128/*
