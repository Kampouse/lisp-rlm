(define (run)
  (let* ((rpc (http-post "https://archival-rpc.mainnet.near.org"
                         "{\"jsonrpc\":\"2.0\",\"id\":\"dontcare\",\"method\":\"query\",\"params\":{\"request_type\":\"call_function\",\"finality\":\"final\",\"account_id\":\"contract.main.burrow.near\",\"method_name\":\"get_margin_account\",\"args_base64\":\"eyJhY2NvdW50X2lkIjoiMTQ1ZTQ4OGNiYmYyNTE1NTY3ODIyZTQ1YWM5Yzg5MTFhYTNmZDgxNjVkOGU0NTU5NjhmMTg2MjU0YWQ4MWZiMyJ9\"}}"))
         (result (json-get-str "result" rpc)))
    result))