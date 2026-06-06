(define (run)
  (let* ((rpc (http-post "https://rpc.mainnet.near.org"
                         "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"query\",\"params\":{\"request_type\":\"call_function\",\"finality\":\"final\",\"account_id\":\"contract.main.burrow.near\",\"method_name\":\"get_margin_accounts_paged\",\"args_base64\":\"eyJmcm9tX2luZGV4IjowLCJsaW1pdCI6MTAwfQ==\"}}"))
         (result (json-get-str "result" rpc)))
    result))