(define (run input)
  (let* (
    (account-id (json-get-str "account_id" input))
    (acct (if (= (str-len account-id) 0) "kampouse.near" account-id))
    
    ;; Fetch all prices (single HTTP call)
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    (lst-p (json-get-str "price" (json-get-str "lst.rhealab.near" prices)))
    (usdt-p (json-get-str "price" (json-get-str "usdt.tether-token.near" prices)))
    (nbtc-p (json-get-str "price" (json-get-str "nbtc.bridge.near" prices)))
    (zec-p (json-get-str "price" (json-get-str "zec.omft.near" prices)))
    
    ;; Fetch Burrow account positions
    (args (str-cat "{\"account_id\":\"" acct "\"}"))
    (account-raw (outlayer/view "contract.main.burrow.near" "get_account" args))
    
    ;; Handle nil/empty results - convert to valid JSON structure
    (account-valid (if (nil? account-raw)
                       "{\"supplied\":[],\"collateral\":[],\"borrowed\":[]}"
                       (if (= (str-len account-raw) 0)
                           "{\"supplied\":[],\"collateral\":[],\"borrowed\":[]}"
                           account-raw)))
    
    (sup-raw (json-get-str "supplied" account-valid))
    (col-raw (json-get-str "collateral" account-valid))
    (bor-raw (json-get-str "borrowed" account-valid))
    
    ;; Ensure arrays even if json-get-str returns nil
    (sup (if (nil? sup-raw) "[]" (if (= (str-len sup-raw) 0) "[]" sup-raw)))
    (col (if (nil? col-raw) "[]" (if (= (str-len col-raw) 0) "[]" col-raw)))
    (bor (if (nil? bor-raw) "[]" (if (= (str-len bor-raw) 0) "[]" bor-raw)))
    
    ;; Build output
    (out (str-cat "{\"account\":\"" acct 
                  "\",\"prices\":{\"lst\":\"" lst-p 
                  "\",\"usdt\":\"" usdt-p 
                  "\",\"nbtc\":\"" nbtc-p 
                  "\",\"zec\":\"" zec-p "\"}"
                  ",\"supplied\":" sup 
                  ",\"collateral\":" col 
                  ",\"borrowed\":" bor "}"))
    )
    out))