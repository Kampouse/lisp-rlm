(define (run)
  (let* (
    ;; Initialize stdout with HTTP call (workaround for P2 stdout bug)
    (_ (http-get "https://api.rhea.finance/list-token-price"))
    (args "{\"account_id\":\"alice.near\"}")
    (account-raw (outlayer/view "contract.main.burrow.near" "get_account" args))
    (is-nil (nil? account-raw))
    )
    (str-cat "{\"is_nil\":" (if is-nil "true" "false") "}")))