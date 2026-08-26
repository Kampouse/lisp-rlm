;; T20 — nested data literals, equality, arity mismatches, and DOCUMENTED PINS
;;
;; Ground rules: (1) no file pins buggy output unless marked as a documented
;; PIN (GAPS.md); (3) arity mismatches SHOULD be errors — the actual
;; interpreter silently accepts them (missing args read as nil-coerced 0 in
;; arithmetic; extra args are dropped). That is a KNOWN bug, GAPS.md
;; "user-fn arity", pinned here as ARITY-PIN — intentional tripwire that
;; flips when an arity check lands.
;; TRUTHINESS-PIN: Num(0) is truthy (Lisp semantics, GAPS.md) — (if 0 ...) takes
;; the then-branch. Flip only if truthiness is deliberately re-specced.
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

;; TRUTHINESS-PIN — 0 is truthy (GAPS.md "Num(0) is truthy")
(println (if 0 "a" "b"))   ; "a"
(println (if "" "a" "b"))  ; "a" — empty string also truthy
(println (if nil "a" "b")) ; "b" — only nil is falsy (plus false)

;; T4-PIN — shared closure cell (documented T4 bug, GAPS.md)
(define (mk) (let ((n 0)) (lambda () (set! n (+ n 1)) n)))
(define c1 (mk))
(define c2 (mk))
(println (c1))  ; 1
(println (c1))  ; 2
(println (c2))  ; 3   T4-PIN: should be 1 with independent cells
(println (c2))  ; 4   T4-PIN: should be 2

;; ARITY-PIN — no arity validation on user fn calls (GAPS.md "user-fn arity")
(define (f2 a b) (+ a b))
(println (f2 1 2))   ; 3 — correct usage
(println (f2 1))     ; 1   ARITY-PIN: should be an error; b reads as nil → 0
(println (f2 1 2 3)) ; 3   ARITY-PIN: should be an error; extra arg dropped
