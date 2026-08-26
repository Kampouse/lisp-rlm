(define (run)
  (let* (
    (_ (http-get "https://api.rhea.finance/list-token-price"))
    (args "{\"account_id\":\"nonexistent12345.near\"}")
    (account (outlayer/view "contract.main.burrow.near" "get_account" args))
    (len (str-len account))
    )
    (str-cat "{\"len\":" (to-string len) "}")))