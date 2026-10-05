;; ADVERSARIAL DREAMED TASK — one notch too hard, delegation required
;; frontier state: ta_prime_sum|ok||rc0|es0|i3
;; dreamed 1791164959.1882870197
(load-file "rlm_runtime.lisp")
(load-file "scripts/rlm-tasks/policy.lisp")
(load-file "scripts/rlm-tasks/q-table.lisp")
(load-file "scripts/rlm-tasks/custom-actions.lisp")
(rlm-set __policy POLICY_ID)
(define (llm-code ctx)
  (llm (build-prompt ctx)))
(begin
  (rlm-set __trace_id "ta_prime_sum_product")
  (rlm-set max_iterations 5)
  (rlm-set __must_decompose true)
  (define (task-verify a) (= a 26500))
  (run-rlm "ADVFRONT Find the sum of all prime numbers below 100, then multiply this sum by the count of prime numbers below 100; store the result via (rlm-set answer ...) then (rlm-set Final true). Solve this by delegating one part via (sub-rlm \"sum of all prime numbers below 100\") and combining.")
  (if (task-verify (rlm-get answer)) nil (re-lesson))
  (write-trace)
  (println (str-concat "RLMDUMP task=ta_prime_sum_product"))
  (println (str-concat "RLMDUMP iterations=" (to-string (rlm-get iteration))))
  (println (str-concat "RLMDUMP answer=" (to-string (rlm-get answer))))
  (println (str-concat "RLMDUMP Final=" (to-string (rlm-get Final))))
)
