;; Live test: Check raw RPC response
(define (run)
  (let* ((args-b64 "eyJhY2NvdW50X2lkIjoiOTQzYWRkYWJkZTc5MTNjNmY1ODA0M2QzNDhlYzc2MzY0MzY4OWI2OGRhMmU3YWIxODZiN2Q3NWUxZDU0NGZmMiJ9")
         (rpc-body (str-cat "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"query\",\"params\":{\"request_type\":\"call_function\",\"finality\":\"final\",\"account_id\":\"contract.main.burrow.near\",\"method_name\":\"get_account\",\"args_base64\":\"" args-b64 "\"}}"))
         (rpc-result (http-post "https://rpc.mainnet.near.org" rpc-body)))
    (str-cat "Status: OK\nLen: " (str-len rpc-result) "\nFirst 100: " (str-slice rpc-result 0 100))))