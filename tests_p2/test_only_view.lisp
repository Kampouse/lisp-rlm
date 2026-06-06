(define (run)
  (let* (
    (args "{\"account_id\":\"alice.near\"}")
    (account-raw (outlayer/view "contract.main.burrow.near" "get_account" args))
    (is-nil (nil? account-raw))
    )
    (str-cat "{\"is_nil\":" (if is-nil "true" "false") "}")))