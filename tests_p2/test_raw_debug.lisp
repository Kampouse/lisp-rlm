(define (run)
  (let* (
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    (meta-raw (json-get-str "meta-pool.near" prices))
    (usdt-raw (json-get-str "usdt.tether-token.near" prices))
    )
    (str-cat "{\"meta_raw\":\"" meta-raw "\",\"usdt_raw\":\"" usdt-raw "\"}")))