(define (run)
  (let* ((rpc (http-post "https://archival-rpc.mainnet.near.org"
                         "{\"jsonrpc\":\"2.0\",\"id\":\"dontcare\",\"method\":\"query\",\"params\":{\"request_type\":\"view_account\",\"finality\":\"final\",\"account_id\":\"contract.main.burrow.near\"}}"))
         (result (json-get-str "result" rpc))
         (balance (json-get-str "amount" result))
         (prices (http-get "https://api.rhea.finance/list-token-price"))
         (near-data (json-get-str "wrap.near" prices))
         (near-usd (json-get-str "price" near-data)))
    (str-cat "Balance: " (str-cat balance " yoctoNEAR | NEAR: $" near-usd))))
