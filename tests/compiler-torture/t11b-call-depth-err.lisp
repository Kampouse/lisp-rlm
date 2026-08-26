;; error: dynamic-dispatch recursion (closure called through a list value)
;; crosses run_compiled_lambda and trips the max_call_depth=256 guard.
;;
;; Boundary probed on this VM: depth 254 is the deepest SUCCESSFUL chain
;; (254 fs-calls + 1 top-level body = 256 crossings); depth 255 is the first
;; to fail with "call depth exceeded", exit 1. Requires the lisp-run 512MB
;; stack thread (commit 13227af) — before it, the native stack aborted the
;; process (exit 134, SIGABRT) around depth ~130, making this guard
;; unreachable.
(define fs
  (list (lambda (n) (if (= n 0) 0 (+ 1 ((car fs) (- n 1)))))))
(println ((car fs) 254))   ; 254 — last depth that fits under the 256 guard
(println ((car fs) 255))   ; dies: call depth exceeded
