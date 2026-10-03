;; ADVERSARIAL DREAMED TASK — one notch too hard, delegation required
;; frontier state: ta_prime_sum|ok||rc0|es0|i2
;; dreamed 1790882733.069578886
(load-file "rlm_runtime.lisp")
(load-file "scripts/rlm-tasks/policy.lisp")
(load-file "scripts/rlm-tasks/q-table.lisp")
(load-file "scripts/rlm-tasks/custom-actions.lisp")
(load-file "scripts/rlm-tasks/rlm_ts.lisp")
(rlm-set __policy POLICY_ID)
(rlm-set __surface "ts")
(define (llm-code ctx)
  (ts->lisp (strip-code-fences (llm (build-prompt-ts ctx)))))
(begin
  (rlm-set __trace_id "ta_prime_sum@ts")
  (rlm-set max_iterations 5)
  (define (task-verify a) (= a 3290))
  (run-rlm "ADVFRONT Given the number 100, compute the sum of all prime numbers less than 50, then multiply that by the count of prime numbers between 50 and 100; store the result via (rlm-set answer ...) then (rlm-set Final true).")
  (if (task-verify (rlm-get answer)) nil (re-lesson))
  (write-trace)
  (println (str-concat "RLMDUMP task=ta_prime_sum@ts"))
  (println (str-concat "RLMDUMP iterations=" (to-string (rlm-get iteration))))
  (println (str-concat "RLMDUMP answer=" (to-string (rlm-get answer))))
  (println (str-concat "RLMDUMP Final=" (to-string (rlm-get Final))))
)
