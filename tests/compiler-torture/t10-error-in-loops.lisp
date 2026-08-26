;; T10 — hard-error propagation out of while/dotimes loop bodies
;;
;; Ground rules: (2) hand-computed constants only, (5) errors inside loops must
;; stop the WHOLE program with exit 1 and the innermost error message.
;; Nesting under test: helper fn → while → helper fn → dotimes → helper fn →
;; division by zero (3 helper levels deep, two loop kinds on the path).
;;
;; Lines 1-2 print before the fatal error; execution then dies inside
;; triple-nested loops with "division by zero" on stderr, exit 1.
;; KNOWN (message inconsistency, see GAPS.md): a LITERAL zero divisor
;; const-folds to "integer overflow in div" ((/ x 0)), while a computed
;; zero divisor ((/ i (- i 3))) reports "division by zero" — same
;; mathematical error, two messages depending on constant folding.

(define (safe-step i)
  (* i 2))

(define (inner-fn i)
  ;; dies when i == 3: runtime div-by-zero via variable operands
  (/ i (- i 3)))

(define (middle-fn n)
  (dotimes (j n)
    (when (= j 3)
      (inner-fn j))))

(define (outer-fn n)
  (let ((i 0))
    (while (< i n)
      (set! i (+ i 1))
      (when (= i 4)
        (middle-fn n)))))

(println (safe-step 21))  ; 42 — sanity line printed before the crash
(println (outer-fn 8))    ; never returns: dies at i=4 → j=3 → (/ 3 0)
