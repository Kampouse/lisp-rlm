;; AUTO-GENERATED @ts twin of t1_sumsq.lisp — edit the BASE task and
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
(rlm-set __trace_id "t1_sumsq@ts")
(define (llm-code ctx)
  (ts->lisp (strip-code-fences (llm (build-prompt-ts ctx)))))
(define (task-verify a) (= a 338350))
(run-rlm "Compute the sum of squares 1+4+9+...+100 by writing and evaluating TypeScript code; store the number in rlm state as answer. When the goal is achieved, finish with rlm_set(\"Final\", true).")
(if (task-verify (rlm-get answer)) nil (re-lesson))
(write-trace)
(println (str-concat "RLMDUMP task=t1_sumsq@ts"))
(println (str-concat "RLMDUMP iterations=" (to-string (rlm-get iteration))))
(println (str-concat "RLMDUMP answer=" (to-string (rlm-get answer))))
(println (str-concat "RLMDUMP Final=" (to-string (rlm-get Final))))
