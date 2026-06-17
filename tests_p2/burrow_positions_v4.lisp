(define (run input)
  (let* (
    (account-id (json-get-str "account_id" input))
    (acct (if (= (str-len account-id) 0) "kampouse.near" account-id))
    
    ;; Fetch all prices FIRST (single HTTP call)
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    (lst-raw (json-get-str "price" (json-get-str "lst.rhealab.near" prices)))
    (usdt-raw (json-get-str "price" (json-get-str "usdt.tether-token.near" prices)))
    (nbtc-raw (json-get-str "price" (json-get-str "nbtc.bridge.near" prices)))
    
    ;; Default USDT to 1 if empty (stable coin)
    (lst-p (if (= (str-len lst-raw) 0) "" lst-raw))
    (usdt-p (if (= (str-len usdt-raw) 0) "1" usdt-raw))
    (nbtc-p (if (= (str-len nbtc-raw) 0) "" nbtc-raw))
    
    (price-json (str-cat "{\"lst\":\"" lst-p "\",\"usdt\":\"" usdt-p "\",\"nbtc\":\"" nbtc-p "\"}"))
    
    ;; Fetch Burrow account - outlayer/view returns "null" string for non-existent
    (args (str-cat "{\"account_id\":\"" acct "\"}"))
    (account-raw (outlayer/view "contract.main.burrow.near" "get_account" args))
    (len (str-len account-raw))
    
    ;; Determine if account exists: len=4 means "null" (non-existent)
    ;; Avoid branching on account-raw directly - just use length
    (has-account (>= len 10))
    
    ;; Format positions: if non-existent, show empty arrays
    (positions (if has-account account-raw "{\"supplied\":[],\"collateral\":[],\"borrowed\":[]}"))
    )
    ;; Output
    (str-cat "{\"account\":\"" acct 
             "\",\"has_account\":" (if has-account "true" "false")
             ",\"prices\":" price-json 
             ",\"positions\":" positions 
             "}")))