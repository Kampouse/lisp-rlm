(define (run)
  (let* ((rpc (http-post "https://archival-rpc.mainnet.near.org"
                         "{\"jsonrpc\":\"2.0\",\"id\":\"dontcare\",\"method\":\"query\",\"params\":{\"request_type\":\"call_function\",\"finality\":\"final\",\"account_id\":\"contract.main.burrow.near\",\"method_name\":\"get_margin_account\",\"args_base64\":\"eyJhY2NvdW50X2lkIjoiOTQzYWRkYWJkZTc5MTNjNmY1ODA0M2QzMzQ4ZWM3NjM2NDM2ODliNjhkYTJlN2FiMTg2YjdkNzVlMWQ1NDRmZjIifQ==\"}}"))
         (result (json-get-str "result" rpc)))
    result))