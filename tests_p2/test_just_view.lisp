(define (run)
  (let* (
    (args "{\"account_id\":\"alice.near\"}")
    (account-raw (outlayer/view "contract.main.burrow.near" "get_account" args))
    (len (str-len account-raw))
    )
    (if (= len 4)
        "{\"status\":\"is_null\"}"
        "{\"status\":\"not_null\"}")))
