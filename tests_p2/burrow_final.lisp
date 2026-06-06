(define (run)
  (let* (
    (account-id (json-get-str "account_id"))
    (acct (if (= (str-len account-id) 0) "kampouse.near" account-id))
    
    ;; Fetch all prices FIRST
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    (lst-raw (json-get-str "price" (json-get-str "lst.rhealab.near" prices)))
    (usdt-raw (json-get-str "price" (json-get-str "usdt.tether-token.near" prices)))
    (nbtc-raw (json-get-str "price" (json-get-str "nbtc.bridge.near" prices)))
    (lst-p (if (= (str-len lst-raw) 0) "" lst-raw))
    (usdt-p (if (= (str-len usdt-raw) 0) "1" usdt-raw))
    (nbtc-p (if (= (str-len nbtc-raw) 0) "" nbtc-raw))
    
    ;; Fetch Burrow account
    (args (str-cat "{\"account_id\":\"" acct "\"}"))
    (account-raw (outlayer/view "contract.main.burrow.near" "get_account" args))
    (len (str-len account-raw))
    (is-null (= len 4))  ;; "null" string is 4 chars
    )
    (if is-null
        ;; No Burrow account - return empty positions
        (str-cat "{\"account\":\"" acct "\",\"prices\":{\"lst\":\"" lst-p "\",\"usdt\":\"" usdt-p "\",\"nbtc\":\"" nbtc-p "\"},\"supplied\":[],\"collateral\":[],\"borrowed\":[]}")
        ;; Has Burrow account - extract positions
        (let* (
          (sup-raw (json-get-str "supplied" account-raw))
          (col-raw (json-get-str "collateral" account-raw))
          (bor-raw (json-get-str "borrowed" account-raw))
          (sup (if (nil? sup-raw) "[]" (if (= (str-len sup-raw) 0) "[]" sup-raw)))
          (col (if (nil? col-raw) "[]" (if (= (str-len col-raw) 0) "[]" col-raw)))
          (bor (if (nil? bor-raw) "[]" (if (= (str-len bor-raw) 0) "[]" bor-raw)))
          (out (str-cat "{\"account\":\"" acct 
                        "\",\"prices\":{\"lst\":\"" lst-p 
                        "\",\"usdt\":\"" usdt-p 
                        "\",\"nbtc\":\"" nbtc-p "\"}"
                        ",\"supplied\":" sup 
                        ",\"collateral\":" col 
                        ",\"borrowed\":" bor "}"))
        )
        out))))