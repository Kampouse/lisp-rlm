;; Debug: return the raw ptr value from ret_area as a number
;; We can't read ret_area from Lisp, so let's call rpc-view twice
;; If the corruption is in the string data, both calls should have it
(define (run)
  (rpc-view "system.near" "get_account_info" "{}" ""))
