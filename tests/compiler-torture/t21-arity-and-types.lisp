;; T21 — user-fn arity validation (round-3 fix 3, landed 2026-08-26)
;;        + arith type-coercion PINS (fix 4 tripwires, not yet fixed)
;;
;; ARITY: all user-fn call paths now hard-error on mismatch —
;; "arity mismatch: <fn> expects N args, got M" (variadic: "at least N").
;; Covers: direct calls, apply, anonymous lambdas, variadic minimums.
;; Choke point: run_compiled_lambda — vm_call_lambda, const-fold inlining,
;; apply/map/filter/reduce, and try/catch thunks all funnel through it.
;;
;; ARITH-PIN: bare + - * / still coerce non-numbers (nil/str/list → 0 in
;; arith) — KNOWN bug, GAPS.md round-3 fix 4. Pinned here as tripwires;
;; flip when num_arith/num_arith_checked/num_cmp return Result.
;; Decision recorded: bare arith = i64/f64 only; string numerics via u128/*.

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

;; ── ARITH-PIN (fix 4) — non-numbers still coerce to 0 in bare arith ──
(println (+ "a" 1))                            ; 1   ARITH-PIN: should err
(println (* (list 1 2) 10))                    ; 0   ARITH-PIN: should err
(println (+ nil 5))                            ; 5   ARITH-PIN: should err
