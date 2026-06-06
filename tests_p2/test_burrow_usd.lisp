(define (run)
  (let* ((rpc (http-post "https://archival-rpc.mainnet.near.org"
                         "{\"jsonrpc\":\"2.0\",\"id\":\"dontcare\",\"method\":\"query\",\"params\":{\"request_type\":\"view_account\",\"finality\":\"final\",\"account_id\":\"contract.main.burrow.near\"}}"))
         (result (json-get-str "result" rpc))
         (balance (json-get-str "amount" result))
         (price-json (http-get "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies=usd"))
         (btc-data (json-get-str "bitcoin" price-json))
         (btc-usd (json-get-str "usd" btc-data)))
    (str-cat "Balance: " (str-cat balance " yoctoNEAR | BTC: $" btc-usd))))
