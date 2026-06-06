(define (run)
  (let* ((rpc (http-post "https://archival-rpc.mainnet.near.org"
                         "{\"jsonrpc\":\"2.0\",\"id\":\"dontcare\",\"method\":\"query\",\"params\":{\"request_type\":\"view_account\",\"finality\":\"final\",\"account_id\":\"contract.main.burrow.near\"}}"))
         (result (json-get-str "result" rpc))
         (amount (json-get-str "amount" result)))
    amount))
