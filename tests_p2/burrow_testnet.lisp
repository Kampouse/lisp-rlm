;; Burrow positions tracker - TESTNET version (hardcoded)
;; Contract: v2.proposal.burrow.testnet

(define (run input)
  (let* (
    (account-id (json-get-str "account_id" input))
    (acct (if (= (str-len account-id) 0) "kampy.testnet" account-id))
    
    ;; Fetch prices
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    (lst-raw (json-get-str "price" (json-get-str "lst.rhealab.near" prices)))
    (usdt-raw (json-get-str "price" (json-get-str "usdt.tether-token.near" prices)))
    (nbtc-raw (json-get-str "price" (json-get-str "nbtc.bridge.near" prices)))
    (lst-p (if (= (str-len lst-raw) 0) "" lst-raw))
    (usdt-p (if (= (str-len usdt-raw) 0) "1" usdt-raw))
    (nbtc-p (if (= (str-len nbtc-raw) 0) "" nbtc-raw))
    
    ;; Fetch Burrow positions from TESTNET
    (args (str-cat "{\"account_id\":\"" acct "\"}"))
    (account-raw (outlayer/view "v2.proposal.burrow.testnet" "get_account" args))
    
    (no-account (if (nil? account-raw) 1 0))
    )
    (if (= no-account 1)
        (str-cat "{\"account\":\"" acct "\",\"network\":\"testnet\",\"error\":\"no_burrow_account\",\"prices\":{\"lst\":\"" lst-p "\",\"usdt\":\"" usdt-p "\",\"nbtc\":\"" nbtc-p "\"},\"supplied\":[],\"collateral\":[],\"borrowed\":[]}")
        (let* (
          (sup-raw (json-get-str "supplied" account-raw))
          (col-raw (json-get-str "collateral" account-raw))
          (bor-raw (json-get-str "borrowed" account-raw))
          (sup (if (nil? sup-raw) "[]" (if (= (str-len sup-raw) 0) "[]" sup-raw)))
          (col (if (nil? col-raw) "[]" (if (= (str-len col-raw) 0) "[]" col-raw)))
          (bor (if (nil? bor-raw) "[]" (if (= (str-len bor-raw) 0) "[]" bor-raw)))
          (out (str-cat "{\"account\":\"" acct 
                        "\",\"network\":\"testnet"
                        "\",\"prices\":{\"lst\":\"" lst-p 
                        "\",\"usdt\":\"" usdt-p 
                        "\",\"nbtc\":\"" nbtc-p "\"}"
                        ",\"supplied\":" sup 
                        ",\"collateral\":" col 
                        ",\"borrowed\":" bor "}"))
        )
        out))))