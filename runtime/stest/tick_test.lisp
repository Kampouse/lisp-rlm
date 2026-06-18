;; harness with tick: register an intention, then tick
(include "harness-p2.lisp")

(define (run input)
  (begin
    (boot)
    (let ((intent (dict "id" "test-1" "type" "one-shot" "action-type" "http-post"
                        "params" (dict "url" "https://httpbin.org/post" "body" "{\"msg\":\"hello\"}")
                        "priority" 10 "cost" 1)))
      (register-intention intent)
      (tick))))
