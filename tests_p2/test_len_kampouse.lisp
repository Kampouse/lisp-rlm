(define (run)
  (let* (
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    (args "{\"account_id\":\"kampouse.near\"}")
    (account-raw (outlayer/view "contract.main.burrow.near" "get_account" args))
    (len (str-len account-raw))
    )
    (str-cat "{\"len\":" (to-string len) "}")))