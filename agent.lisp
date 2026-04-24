;; Minimal test agent
(define agent-name "Gork")

(if (file-exists? "strategies.lisp")
  (load-file "strategies.lisp"))

(if (file-exists? "memory/state.lisp")
  (load-file "memory/state.lisp"))

(if (not (rlm-get "booted"))
  (begin
    (rlm-set "booted" true)
    (rlm-set "total_tasks" 0)
    (rlm-set "successes" 0)
    (println (str-concat agent-name " booted"))))

;; Define route but don't call it
(define (route message)
  (println (str-concat "Routing: " message))
  (final message))

(println "agent.lisp loaded")
