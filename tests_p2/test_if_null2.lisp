(define (run)
  (let* (
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    (args "{\"account_id\":\"alice.near\"}")
    (account-raw (outlayer/view "contract.main.burrow.near" "get_account" args))
    (len (str-len account-raw))
    (is-null (= len 4))
    )
    ;; Return early if null, otherwise return default
    (if is-null
        "{\"status\":\"null_detected\"}"
        "{\"status\":\"has_account\"}")))