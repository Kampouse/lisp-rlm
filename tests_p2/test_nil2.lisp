(define (run)
  (let* (
    (account-id (json-get-str "account_id" input))
    (acct (if (= (str-len account-id) 0) "kampouse.near" account-id))
    (args (str-cat "{\"account_id\":\"" acct "\"}"))
    (account-raw (outlayer/view "contract.main.burrow.near" "get_account" args))
    ;; Debug: show what we got
    (dbg (str-cat "{\"acct\":\"" acct "\",\"args\":\"" args "\",\"raw_nil\":" (if (nil? account-raw) "true" "false") "}"))
    )
    dbg))