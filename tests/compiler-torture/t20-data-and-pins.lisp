;; T20 — nested data literals, equality, arity mismatches, and DOCUMENTED PINS
;;
;; Ground rules: (1) no file pins buggy output unless marked as a documented
;; PIN (GAPS.md); (3) arity mismatches ARE errors (round-3 fix 3, flipped
;; 2026-08-26) — "arity mismatch: fn expects N args, got M".
;; TRUTHINESS-PIN (flipped, GAPS.md round 3 fix 1): numeric zero is FALSY —
;; Num(0) and Float(0.0) both take the else-branch. Deliberate re-spec
;; (was: 0 truthy, Lisp-1 style). "" and '() REMAIN truthy by decision.
;; T4-PIN: closures returned from the same factory SHARE one cell (GAPS.md
;; T4) — c2 sees c1's increments. Correct semantics: independent cells
;; (1 2 1 2). Pinned as the T4 tripwire.

(println (= (list 1 (list 2 3) 4) (list 1 (list 2 3) 4))) ; true — deep equality
(println (= (list 1 (list 2 3)) (list 1 (list 2 4))))     ; false — inner differs
(println (= (list 1 2) (list 1 2 3)))                     ; false — length differs
(println '(1 (2 (3 4))))                                  ; (1 (2 (3 4))) — nested literal
(println (len '(1 (2 (3 4)))))                            ; 2 — outer cons cells only
(println ''x)                                             ; (quote x) — double quote nests
(println (car ''x))                                       ; quote — head symbol of (quote x)
(println (= nil nil))                                     ; true
(println (= nil '()))                                     ; false — nil and empty list are DISTINCT
(println (nil? nil))                                      ; true
(println (nil? '()))                                      ; false — '() is a List, not nil

;; TRUTHINESS-PIN (flipped, round 3) — numeric zero is falsy
(println (if 0 "a" "b"))     ; "b" — 0 falsy now
(println (if 0.0 "a" "b"))   ; "b" — 0.0 falsy too
(println (if "" "a" "b"))    ; "a" — empty string stays TRUTHY
(println (if nil "a" "b"))   ; "b" — nil falsy
(println (if '() "a" "b"))   ; "a" — empty list stays TRUTHY
(println (if 7 "a" "b"))     ; "a" — nonzero num truthy
(println (not 0))            ; true

;; T4-PIN — shared closure cell (documented T4 bug, GAPS.md)
(define (mk) (let ((n 0)) (lambda () (set! n (+ n 1)) n)))
(define c1 (mk))
(define c2 (mk))
(println (c1))  ; 1
(println (c1))  ; 2
(println (c2))  ; 3   T4-PIN: should be 1 with independent cells
(println (c2))  ; 4   T4-PIN: should be 2

;; ARITY (flipped, round-3 fix 3) — user-fn calls hard-error on mismatch.
;; Was ARITY-PIN: missing args read as nil (arith-coerced to 0), extra args
;; silently dropped. Now: "arity mismatch: f2 expects 2 args, got N".
(define (f2 a b) (+ a b))
(println (f2 1 2))                             ; 3 — correct usage
(println (try (f2 1) (catch e "err-missing"))) ; err-missing — b no longer nil→0
(println (try (f2 1 2 3) (catch e "err-extra"))) ; err-extra — no longer dropped
