(define (run input)
  (let* (
    (account-id (json-get-str "account_id" input))
    (acct (if (= (str-len account-id) 0) "kampouse.near" account-id))
    
    ;; Fetch all prices FIRST
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    (lst-raw (json-get-str "price" (json-get-str "lst.rhealab.near" prices)))
    (usdt-raw (json-get-str "price" (json-get-str "usdt.tether-token.near" prices)))
    (nbtc-raw (json-get-str "price" (json-get-str "nbtc.bridge.near" prices)))
    (lst-p (if (= (str-len lst-raw) 0) "" lst-raw))
    (usdt-p (if (= (str-len usdt-raw) 0) "1" usdt-raw))
    (nbtc-p (if (= (str-len nbtc-raw) 0) "" nbtc-raw))
    
    (price-json (str-cat "{\"lst\":\"" lst-p "\",\"usdt\":\"" usdt-p "\",\"nbtc\":\"" nbtc-p "\"}"))
    
    ;; Fetch Burrow account
    (args (str-cat "{\"account_id\":\"" acct "\"}"))
    (account-raw (outlayer/view "contract.main.burrow.near" "get_account" args))
    (len (str-len account-raw))
    )
    ;; Build base output
    (str-cat "{\"account\":\"" acct "\",\"prices\":" price-json ",\"has_account\":" (if (= len 4) "false" "true") ",\"len\":" (to-string len) "}")))