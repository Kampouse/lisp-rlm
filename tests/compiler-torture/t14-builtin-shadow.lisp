;; T14 — shadowing builtins: let-binding over builtin names, user fns with
;; builtin names, scoping and restoration
;;
;; Ground rules: (2) constants hand-computed; (4) shadowing a builtin must be
;; SCOPED — after the let/define the builtin's original meaning is what the
;; rest of the file observes (or the user define persists at top level —
;; both directions pinned below).
;; KNOWN (GAPS.md, torture r2): compiled arithmetic silently coerces
;; non-number operands to 0 ((+ "a" 1) → 1, (* (list 1 2) 10) → 0), so a
;; user-defined (define (car x) (* x 10)) applied to a list returns 0
;; instead of erroring. Not pinned here — see GAPS.md.

;; 1. let-shadow a builtin as a VALUE; scope restores after
(let ((car 5))
  (println car)               ; 5
  (let ((car 6))
    (println car))            ; 6  — nested shadow wins
  (println car))              ; 5  — inner shadow popped
(println (car (list 1 2)))    ; 1  — builtin restored after scope exit

;; 2. deep shadow + use alongside the same builtin via alias-free rebind
(let ((str-length 99))
  (println str-length)        ; 99
  (println (+ str-length 1))) ; 100
(println (str-length "abcd")) ; 4  — restored

;; 3. define a function with a builtin name — the user fn wins afterwards
(define (twice x) (* x 2))
(println (twice 21))          ; 42 — sanity
(define (max a b) (- a b))    ; user max: subtraction, not maximum
(println (max 10 3))          ; 7  — user define wins over builtin max

;; 4. lambda parameter shadowing a builtin name
(define (use-len-as-param len)
  (+ len 1))
(println (use-len-as-param 41)) ; 42
(println (len (list 9 9 9 9)))  ; 4 — builtin len intact outside
