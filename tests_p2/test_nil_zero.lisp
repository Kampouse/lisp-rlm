(define (run)
  (let* (
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    (args "{\"account_id\":\"alice.near\"}")
    (account-raw (outlayer/view "contract.main.burrow.near" "get_account" args))
    (is-nil (nil? account-raw))
    (is-zero (= (str-len account-raw) 0))
    )
    (str-cat "{\"nil\":" (if is-nil "true" "false") ",\"zero\":" (if is-zero "true" "false") "}")))