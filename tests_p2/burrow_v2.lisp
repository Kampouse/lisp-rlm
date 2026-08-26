(define (run input)
  (let* (
    (account-id (json-get-str "account_id" input))
    (acct (if (= (str-len account-id) 0)
              "kampouse.near"
              account-id))
    
    ;; Fetch all prices
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    (lst-p (json-get-str "price" (json-get-str "lst.rhealab.near" prices)))
    (usdt-p (json-get-str "price" (json-get-str "usdt.tether-token.near" prices)))
    (nbtc-p (json-get-str "price" (json-get-str "nbtc.bridge.near" prices)))
    
    ;; Fetch Burrow account
    (args (str-cat "{\"account_id\":\"" acct "\"}"))
    (account-raw (outlayer/view "contract.main.burrow.near" "get_account" args))
    
    ;; Handle nil - convert to empty JSON object
    (account (if (nil? account-raw) "{}" account-raw))
    
    ;; Extract positions with nil safety
    (sup-raw (json-get-str "supplied" account))
    (col-raw (json-get-str "collateral" account))
    (bor-raw (json-get-str "borrowed" account))
    (sup (if (nil? sup-raw) "[]" sup-raw))
    (col (if (nil? col-raw) "[]" col-raw))
    (bor (if (nil? bor-raw) "[]" bor-raw))
    
    (out (str-cat "{\"account\":\"" acct 
                  "\",\"prices\":{\"lst\":\"" lst-p "\",\"usdt\":\"" usdt-p "\",\"nbtc\":\"" nbtc-p "\"}"
                  ",\"supplied\":" sup ",\"collateral\":" col ",\"borrowed\":" bor "}"))
    )
    out))