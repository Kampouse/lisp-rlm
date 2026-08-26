;; Test rpc-view + env-var: read signer key from env, call a view function
(define (run)
  (rpc-view "outlayer.near" "get_project" "{\"project_id\":\"\"}" "final"))
