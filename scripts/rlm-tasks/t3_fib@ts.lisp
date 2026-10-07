;; AUTO-GENERATED @ts twin of t3_fib.lisp — edit the BASE task and
;; regenerate: python3 scripts/gen-ts-twins.py  (do not hand-edit)
;; RLM nightly task — run by scripts/rlm-nightly.sh via lisp-run.
;; Policy (grammar etc) is DATA in policy.lisp; llm-code wrapper
;; injects it; every run emits a trace world to data/rlm/traces/.
(load-file "rlm_runtime.lisp")
(load-file "scripts/rlm-tasks/policy.lisp")
(load-file "scripts/rlm-tasks/q-table.lisp")
(load-file "scripts/rlm-tasks/custom-actions.lisp")
(load-file "scripts/rlm-tasks/rlm_ts.lisp")
(rlm-set __policy POLICY_ID)
(rlm-set __surface "ts")
(rlm-set __trace_id "t3_fib@ts")
(define (llm-code ctx)
  (ts->lisp (strip-code-fences (llm (build-prompt-ts ctx)))))
(define (task-verify a) (equal? a (list 1 1 2 3 5 8 13)))
(run-rlm "Build the list of Fibonacci numbers up to 13, that is 1 1 2 3 5 8 13, by writing and evaluating TypeScript code; store the list as answer. When achieved, finish with rlm_set(\"Final\", true).")
(retry-with-feedback task-verify "(list 1 1 2 3 5 8 13)" 2)
(if (task-verify (rlm-get answer)) nil (re-lesson))
(write-trace)
(println (str-concat "RLMDUMP task=t3_fib@ts"))
(println (str-concat "RLMDUMP iterations=" (to-string (rlm-get iteration))))
(println (str-concat "RLMDUMP answer=" (to-string (rlm-get answer))))
(println (str-concat "RLMDUMP Final=" (to-string (rlm-get Final))))
