(define (run)
  (let* (
    (args "{\"account_id\":\"alice.near\"}")
    (account-raw (outlayer/view "contract.main.burrow.near" "get_account" args))
    (len (str-len account-raw))
    )
    ;; Output raw - no if, no branch
    (str-cat "{\"len\":" (to-string len) ",\"raw\":\"" account-raw "\"}")))