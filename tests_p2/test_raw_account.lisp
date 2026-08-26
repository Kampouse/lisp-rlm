(define (run)
  (let* (
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    (args "{\"account_id\":\"alice.near\"}")
    (account-raw (outlayer/view "contract.main.burrow.near" "get_account" args))
    (len (str-len account-raw))
    )
    ;; Always return - test if conditional is the issue
    (str-cat "{\"len\":" (to-string len) ",\"account_raw\":\"" account-raw "\"}")))