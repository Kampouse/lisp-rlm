(define (run)
  (let* (
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    (nbtc (json-get-str "price" (json-get-str "nbtc.bridge.near" prices)))
    (result (str-cat "{\"nbtc\":\"" nbtc "\"}"))
    )
    result))
