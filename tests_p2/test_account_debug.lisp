(define (run)
  (let* ((payload "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"query\",\"params\":{\"request_type\":\"call_function\",\"finality\":\"final\",\"account_id\":\"contract.main.burrow.near\",\"method_name\":\"get_account\",\"args_base64\":\"eyJhY2NvdW50X2lkIjoiOTQzYWRkYWJkZTc5MTNjNmY1ODA0M2QzMzQ4ZWM3NjM2NDM2ODliNjhkYTJlN2FiMTg2YjdkNzVlMWQ1NDRmZjIifQ==\"}}")
         (result (http-post "https://rpc.mainnet.near.org" payload)))
    result))