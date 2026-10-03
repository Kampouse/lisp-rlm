;; return-effect-once.lisp — tripwire for double-eval in return/log positions
;; Each (bump k) increments a decimal-string counter and returns the new count.
;; A call site that emits its argument exactly once leaves the counter at "1";
;; the historical double-emit bug leaves it at "2".
;; Run via near-mock scenario: tests/return-effect-once.json
;;
;; near/return is str→nil at the Lisp surface (typing/types.rs); its emitter
;; else-branch (non-string) is reachable from the TS frontend and is covered
;; by the same fix — fixed by inspection here.

;; ── helpers (must precede use) ──
(define (bump k)
  (near/storage_set k (u128/add (default (near/storage_get k) "0") "1"))
  (default (near/storage_get k) "0"))

;; ── test arms ──
;; arm A: effect directly in near/return_str position — was BUGGY (2)
(define (t-ret-str-direct) (near/return_str (bump "a")))

;; arm B: control — known-safe let convention — always 1
(define (t-ret-str-let)
  (let ((v (bump "b"))) (near/return_str v)))

;; arm D: near/return string path (only Lisp-reachable path) — expect 1
(define (t-ret-direct) (near/return (bump "d")))

;; arm E: effect directly in near/log position — was BUGGY (2)
(define (t-log-direct) (near/log (bump "e")))

;; arm F: control for log — always 1
(define (t-log-let)
  (let ((v (bump "f"))) (near/log v)))

;; ── views ──
(define (peek) (near/return_str (default (near/storage_get (near/json_get_str "k")) "")))

(export "t-ret-str-direct" t-ret-str-direct #f)
(export "t-ret-str-let" t-ret-str-let #f)
(export "t-ret-direct" t-ret-direct #f)
(export "t-log-direct" t-log-direct #f)
(export "t-log-let" t-log-let #f)
(export "peek" peek #t)
