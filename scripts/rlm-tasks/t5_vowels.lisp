;; RLM nightly task — run by scripts/rlm-nightly.sh via lisp-run.
;; Policy (grammar etc) is DATA in policy.lisp; llm-code wrapper
;; injects it; every run emits a trace world to data/rlm/traces/.
(load-file "rlm_runtime.lisp")
(load-file "scripts/rlm-tasks/policy.lisp")
(load-file "scripts/rlm-tasks/q-table.lisp")
(load-file "scripts/rlm-tasks/custom-actions.lisp")
(rlm-set __policy POLICY_ID)
(rlm-set __trace_id "t5_vowels")
(define (llm-code ctx)
  (llm (build-prompt ctx)))
(define (task-verify a) (= a 5))
(run-rlm "Count the vowels in the string 'precaution' by writing and evaluating lisp code; store the count as answer. When achieved, evaluate (final true).")
(if (task-verify (rlm-get answer)) nil (re-lesson))
(write-trace)
(println (str-concat "RLMDUMP task=t5_vowels"))
(println (str-concat "RLMDUMP iterations=" (to-string (rlm-get iteration))))
(println (str-concat "RLMDUMP answer=" (to-string (rlm-get answer))))
(println (str-concat "RLMDUMP Final=" (to-string (rlm-get Final))))
