(define (run)
  (let* ((rpc (http-post "https://archival-rpc.mainnet.near.org"
                         "{\"jsonrpc\":\"2.0\",\"id\":\"dontcare\",\"method\":\"query\",\"params\":{\"request_type\":\"call_function\",\"finality\":\"final\",\"account_id\":\"contract.main.burrow.near\",\"method_name\":\"get_margin_account\",\"args_base64\":\"eyJhY2NvdW50X2lkIjoiMHg5NDNhZGRhYmRlNzkxM2M2ZjU4MDQzZDMzNDhlYzc2MzY0MzY4OWI2OGRhMmU3YWIxODZiN2Q3NWUxZDU0NGZmMiJ9\"}}"))
         (result (json-get-str "result" rpc)))
    result))