(define (run)
  (let* (
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    (nbtc (json-get-str "price" (json-get-str "nbtc.bridge.near" prices)))
    (zec (json-get-str "price" (json-get-str "zec.omft.near" prices)))
    (usdt (json-get-str "price" (json-get-str "usdt.tether-token.near" prices)))
    (result (str-cat "{" nbtc "," zec "," usdt "}"))
    )
    result))
