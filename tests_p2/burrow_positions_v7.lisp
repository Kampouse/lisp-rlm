(define (run input)
  (let* (
    (account-id (json-get-str "account_id" input))
    (acct (if (= (str-len account-id) 0) "kampouse.near" account-id))
    
    ;; Fetch prices from Rhea
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    (lst-raw (json-get-str "price" (json-get-str "lst.rhealab.near" prices)))
    (usdt-raw (json-get-str "price" (json-get-str "usdt.tether-token.near" prices)))
    
    ;; Fetch Burrow account positions
    (args (str-cat "{\"account_id\":\"" acct "\"}"))
    (account-raw (near-view "contract.main.burrow.near" "get_account" args "final"))
    
    ;; Return result
    (out (str-cat "{\"account\":\"" acct "\",\"prices\":{\"lst\":\"" lst-raw "\",\"usdt\":\"" usdt-raw "\"},\"raw\":\"" account-raw "\"}"))
    )
    out))
