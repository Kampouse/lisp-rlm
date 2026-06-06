(define (run)
  (let* (
    ;; Fetch all prices in one call
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    (nbtc-p (json-get-str "price" (json-get-str "nbtc.bridge.near" prices)))
    (zec-p (json-get-str "price" (json-get-str "zec.omft.near" prices)))
    (usdt-p (json-get-str "price" (json-get-str "usdt.tether-token.near" prices)))
    (stnear-p (json-get-str "price" (json-get-str "meta-pool.near" prices)))
    (xrhea-p (json-get-str "price" (json-get-str "xtoken.rhealab.near" prices)))
    (burrow-p (json-get-str "price" (json-get-str "token.burrow.near" prices)))
    (lst-p (json-get-str "price" (json-get-str "lst.rhealab.near" prices)))
    (usdce-p (json-get-str "price" (json-get-str "a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48.factory.bridge.near" prices)))
    (weth-p (json-get-str "price" (json-get-str "2260fac5e5542a773aa44fbcfedf7c193bc2c599.factory.bridge.near" prices)))
    
    ;; Build prices JSON object
    (prices-0 (str-cat "{\"nbtc.bridge.near\":\"" nbtc-p "\""))
    (prices-1 (str-cat prices-0 ",\"zec.omft.near\":\"" zec-p "\""))
    (prices-2 (str-cat prices-1 ",\"usdt.tether-token.near\":\"" usdt-p "\""))
    (prices-3 (str-cat prices-2 ",\"meta-pool.near\":\"" stnear-p "\""))
    (prices-4 (str-cat prices-3 ",\"xtoken.rhealab.near\":\"" xrhea-p "\""))
    (prices-5 (str-cat prices-4 ",\"token.burrow.near\":\"" burrow-p "\""))
    (prices-6 (str-cat prices-5 ",\"lst.rhealab.near\":\"" lst-p "\""))
    (prices-7 (str-cat prices-6 ",\"a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48.factory.bridge.near\":\"" usdce-p "\""))
    (prices-obj (str-cat prices-7 ",\"2260fac5e5542a773aa44fbcfedf7c193bc2c599.factory.bridge.near\":\"" weth-p "\"}"))
    
    ;; Fetch Burrow account positions via outlayer/view (240 instr vs 39K for http-post)
    (account (outlayer/view "contract.main.burrow.near" "get_account" "{\"account_id\":\"kampouse.near\"}"))
    (supplied (json-get-str "supplied" account))
    (collateral (json-get-str "collateral" account))
    (borrowed (json-get-str "borrowed" account))
    
    ;; Combine into output
    (out (str-cat "{\"prices\":" prices-obj ",\"supplied\":" supplied ",\"collateral\":" collateral ",\"borrowed\":" borrowed "}"))
    )
    out))