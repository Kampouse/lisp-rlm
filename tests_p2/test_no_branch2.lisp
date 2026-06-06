(define (run)
  (let* (
    (args "{\"account_id\":\"alice.near\"}")
    (account-raw (outlayer/view "contract.main.burrow.near" "get_account" args))
    (len (str-len account-raw))
    (is-null (= len 4))
    )
    (str-cat "{\"len\":" (to-string len) ",\"is_null\":" (if is-null "true" "false") "}")))