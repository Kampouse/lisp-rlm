;; ADVERSARIAL DREAMED TASK — one notch too hard, delegation required
;; frontier state: decsmoke|ok||rc0|es0|i1
;; dreamed 1789915890.5050261021
(load-file "rlm_runtime.lisp")
(load-file "scripts/rlm-tasks/policy.lisp")
(load-file "scripts/rlm-tasks/q-table.lisp")
(load-file "scripts/rlm-tasks/custom-actions.lisp")
(rlm-set __policy POLICY_ID)
(define (llm-code ctx)
  (llm (build-prompt ctx)))
(begin
  (rlm-set __trace_id "ta_prime_sum")
  (rlm-set max_iterations 5)
  (rlm-set __must_decompose true)
  (define (task-verify a) (= a 30))
  (run-rlm "ADVFRONT Find the sum of all prime numbers less than 10, then multiply that sum by 3; store the final result via (rlm-set answer ...) then (rlm-set Final true). Solve this by delegating one part via (sub-rlm \"sum of all prime numbers less than 10\") and combining.")
(retry-with-feedback task-verify "30" 2)
  (if (task-verify (rlm-get answer)) nil (re-lesson))
  (write-trace)
  (println (str-concat "RLMDUMP task=ta_prime_sum"))
  (println (str-concat "RLMDUMP iterations=" (to-string (rlm-get iteration))))
  (println (str-concat "RLMDUMP answer=" (to-string (rlm-get answer))))
  (println (str-concat "RLMDUMP Final=" (to-string (rlm-get Final))))
)
