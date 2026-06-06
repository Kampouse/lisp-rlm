(define (run)
  (let* (
    (account-id (json-get-str "account_id"))
    (acct (if (= (str-len account-id) 0)
              "kampouse.near"
              account-id))
    (args (str-cat "{\"account_id\":\"" acct "\"}"))
    (account (outlayer/view "contract.main.burrow.near" "get_account" args))
    (result (if (nil? account)
                "{\"error\":\"nil_account\"}"
                (if (= (str-len account) 0)
                    "{\"error\":\"empty_account\"}"
                    (str-cat "{\"ok\":\"" acct "\",\"len\":" (to-string (str-len account)) "}"))))
    )
    result))