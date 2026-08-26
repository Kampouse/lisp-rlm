(define (run input)
  (let* (
    (account-id (json-get-str "account_id" input))
    (acct (if (= (str-len account-id) 0)
              "kampouse.near"
              account-id))
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    (lst-p (json-get-str "price" (json-get-str "lst.rhealab.near" prices)))
    (args (str-cat "{\"account_id\":\"" acct "\"}"))
    (account (outlayer/view "contract.main.burrow.near" "get_account" args))
    (debug (str-cat "{\"account_id_raw\":\"" account-id "\",\"account\":\"" acct "\",\"lst_price\":\"" lst-p "\",\"args\":\"" args "\",\"account_result\":\"" account "\"}"))
    )
    debug))