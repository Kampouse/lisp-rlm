;; RLM nightly task — run by scripts/rlm-nightly.sh via lisp-run.
;; llm-code wrapper: prepend the lisp-rlm grammar so the model writes
;; valid surface (it defaults to Common Lisp: loop/incf/etc = compile ERR).
(load-file "rlm_runtime.lisp")
(define GRAMMAR
  "Write lisp-rlm code ONLY. Available: define (define (f x) body), let ((v e)) , while cond, dotimes (i n), set!, if, + - * / mod abs, = < > <= >=, and or not, list car cdr len map lambda, str-cat str-length str-substring str-upcase str-downcase str-contains, u128/add u128/mul u128/div u128/lt (string args), rlm-set rlm-get final. NO loop-for, NO incf, NO Common Lisp. EXACT loop shapes: (while (< i n) (set! i (+ i 1))), (dotimes (i 100) (set! sum (+ sum 1))) — dotimes binding is (i 100) with ONE pair of parens, body forms follow directly. Integers only for numbers. Output ONE lisp form only.")
(define (llm-code ctx)
  (llm (str-concat GRAMMAR "\n\nTASK CONTEXT:\n" ctx)))
(run-rlm "Compute 2 to the power 16 by writing and evaluating lisp code; store it as answer. When achieved, evaluate (final true).")
(println (str-concat "RLMDUMP task=t4_pow"))
(println (str-concat "RLMDUMP iterations=" (to-string (rlm-get iteration))))
(println (str-concat "RLMDUMP answer=" (to-string (rlm-get answer))))
(println (str-concat "RLMDUMP Final=" (to-string (rlm-get Final))))
(println (str-concat "RLMDUMP exec_log=" (to-string (rlm-get exec_log))))
