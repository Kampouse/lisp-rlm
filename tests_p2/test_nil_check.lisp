(define (run)
  (let* (
    (_ (http-get "https://api.rhea.finance/list-token-price"))
    (args "{\"account_id\":\"nonexistent12345.near\"}")
    (account (outlayer/view "contract.main.burrow.near" "get_account" args))
    (is-nil (nil? account))
    )
    (if is-nil
        "{\"found\":false}"
        (str-cat "{\"found\":true,\"len\":" (to-string (str-len account)) "}"))))