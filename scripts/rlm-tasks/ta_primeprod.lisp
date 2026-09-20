;; ADVERSARIAL DREAMED TASK — one notch too hard, delegation required
;; frontier state: decsmoke|ok||rc0|es0|i1
;; dreamed 1789906432.9122169018
(load-file "rlm_runtime.lisp")
(load-file "scripts/rlm-tasks/policy.lisp")
(load-file "scripts/rlm-tasks/q-table.lisp")
(load-file "scripts/rlm-tasks/custom-actions.lisp")
(rlm-set __policy POLICY_ID)
(define (llm-code ctx)
  (llm (build-prompt ctx)))
(begin
  (rlm-set __trace_id "ta_primeprod")
  (rlm-set max_iterations 5)
  (rlm-set __must_decompose true)
  (define (task-verify a) (= a 352))
  (run-rlm "ADVFRONT Given (list 2 3 5 7 11), compute the product of the primes greater than 3, then add the count of primes less than 5; store the combined result via (rlm-set answer ...) then (rlm-set Final true). Solve this by delegating one part via (sub-rlm \"product of primes greater than 3 in (list 2 3 5 7 11)\") and combining.")
  (if (task-verify (rlm-get answer)) nil (rlm-set Final nil))
  (write-trace)
  (println (str-concat "RLMDUMP task=ta_primeprod"))
  (println (str-concat "RLMDUMP iterations=" (to-string (rlm-get iteration))))
  (println (str-concat "RLMDUMP answer=" (to-string (rlm-get answer))))
  (println (str-concat "RLMDUMP Final=" (to-string (rlm-get Final))))
)
