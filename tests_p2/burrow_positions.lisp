(define (run input)
  (let* (
    (account-id (json-get-str "account_id" input))
    (acct (if (= (str-len account-id) 0)
              "kampouse.near"
              account-id))
    
    ;; Fetch all prices in one call
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    (nbtc-p (json-get-str "price" (json-get-str "nbtc.bridge.near" prices)))
    (zec-p (json-get-str "price" (json-get-str "zec.omft.near" prices)))
    (usdt-p (json-get-str "price" (json-get-str "usdt.tether-token.near" prices)))
    (lst-p (json-get-str "price" (json-get-str "lst.rhealab.near" prices)))
    
    ;; Fetch Burrow account positions
    (args (str-cat "{\"account_id\":\"" acct "\"}"))
    (account (outlayer/view "contract.main.burrow.near" "get_account" args))
    
    ;; Handle nil account (non-existent or no positions)
    (account-ok (if (nil? account) "{}" account))
    (supplied (json-get-str "supplied" account-ok))
    (collateral (json-get-str "collateral" account-ok))
    (borrowed (json-get-str "borrowed" account-ok))
    
    ;; Handle nil values for positions
    (sup (if (or (nil? supplied) (= (str-len supplied) 0)) "[]" supplied))
    (col (if (or (nil? collateral) (= (str-len collateral) 0)) "[]" collateral))
    (bor (if (or (nil? borrowed) (= (str-len borrowed) 0)) "[]" borrowed))
    
    ;; Build output
    (out (str-cat "{\"account\":\"" acct 
                  "\",\"prices\":{\"nbtc\":\"" nbtc-p 
                  "\",\"zec\":\"" zec-p 
                  "\",\"usdt\":\"" usdt-p 
                  "\",\"lst\":\"" lst-p 
                  "\"},\"supplied\":" sup 
                  ",\"collateral\":" col 
                  ",\"borrowed\":" bor "}"))
    )
    out))