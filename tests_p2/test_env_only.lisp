;; Test rpc-view + env-var: read signer key from env, return it as output
(define (run)
  (env-var "HOME"))
