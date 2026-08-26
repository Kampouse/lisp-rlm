(define (run)
  (let* (
    ;; Extract account_id from stdin JSON (implicit)
    (account-id (json-get-str "account_id" input))
    
    ;; Handle empty/nil account_id
    (acct (if (= (str-len account-id) 0)
              "kampouse.near"
              account-id))
    
    ;; Fetch all prices in one call
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    (nbtc-p (json-get-str "price" (json-get-str "nbtc.bridge.near" prices)))
    (usdt-p (json-get-str "price" (json-get-str "usdt.tether-token.near" prices)))
    (lst-p (json-get-str "price" (json-get-str "lst.rhealab.near" prices)))
    
    ;; Build prices JSON object
    (prices-obj (str-cat "{\"nbtc\":\"" nbtc-p "\",\"usdt\":\"" usdt-p "\",\"lst\":\"" lst-p "\"}"))
    
    ;; Fetch Burrow account positions via outlayer/view
    (args (str-cat "{\"account_id\":\"" acct "\"}"))
    (account (outlayer/view "contract.main.burrow.near" "get_account" args))
    (supplied (json-get-str "supplied" account))
    (collateral (json-get-str "collateral" account))
    (borrowed (json-get-str "borrowed" account))
    
    ;; Combine into output
    (out (str-cat "{\"account\":\"" acct "\",\"prices\":" prices-obj ",\"supplied\":" supplied ",\"collateral\":" collateral ",\"borrowed\":" borrowed "}"))
    )
    out))