(define (run)
  (let* (
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    (nbtc (json-get-str "price" (json-get-str "nbtc.bridge.near" prices)))
    (weth (json-get-str "price" (json-get-str "2260fac5e5542a773aa44fbcfedf7c193bc2c599.factory.bridge.near" prices)))
    (result (str-cat "{\"nbtc\":\"" nbtc "\",\"weth\":\"" weth "\"}"))
    )
    result))
