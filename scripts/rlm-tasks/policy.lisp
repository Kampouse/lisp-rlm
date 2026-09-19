;; ============================================================
;; Exploration policy — versioned DATA, loaded by task templates.
;; The dream layer (rlm-replay.py + DREAM tasks) proposes edits;
;; a new policy deploys only if replay-score ≥ incumbent on the
;; frozen trace pool (data/rlm/traces/).
;; ============================================================
(define POLICY_ID "p3-grammar-shapes")
(define GRAMMAR
  "Write lisp-rlm code ONLY. Available: define (define (f x) body), let ((v e)) , while cond, dotimes (i n), set!, if, + - * / mod abs, = < > <= >=, and or not, list car cdr len map lambda, str-cat str-length str-substring str-upcase str-downcase str-contains, u128/add u128/mul u128/div u128/lt (string args), rlm-set rlm-get final. NO loop-for, NO incf, NO Common Lisp. EXACT loop shapes: (while (< i n) (set! i (+ i 1))), (dotimes (i 100) (set! sum (+ sum 1))) — dotimes binding is (i 100) with ONE pair of parens, body forms follow directly. Integers only for numbers. Output ONE lisp form only.")
