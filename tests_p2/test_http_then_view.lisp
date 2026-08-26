(define (run)
  (let* (
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    (account (outlayer/view "contract.main.burrow.near" "get_account" "{\"account_id\":\"kampouse.near\"}"))
    )
    (str-cat "{\"len_p\":" (to-string (str-len prices)) ",\"len_a\":" (to-string (str-len account)) "}")))
