;; Test rpc-view with a simple contract that returns account info
(define (run)
  (rpc-view "system.near" "get_account_info" "{}" ""))
