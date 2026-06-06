;; Live test: Debug byte array decode
(define (run)
  (let* ((account "943addabde7913c6f58043d348ec763643689b68da2e7ab186b7d75e1d544ff2")
         (args-b64 "eyJhY2NvdW50X2lkIjoiOTQzYWRkYWJkZTc5MTNjNmY1ODA0M2QzNDhlYzc2MzY0MzY4OWI2OGRhMmU3YWIxODZiN2Q3NWUxZDU0NGZmMiJ9")
         
         ;; Query NEAR RPC for Burrow account
         (rpc-body (str-cat "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"query\",\"params\":{\"request_type\":\"call_function\",\"finality\":\"final\",\"account_id\":\"contract.main.burrow.near\",\"method_name\":\"get_account\",\"args_base64\":\"" args-b64 "\"}}"))
         (rpc-result (http-post "https://rpc.mainnet.near.org" rpc-body))
         (raw-bytes (json-get-str "result" rpc-result))
         (decoded (json-decode-bytes raw-bytes)))
    (str-cat "RPC len: " (str-len raw-bytes) "\nDecoded len: " (str-len decoded))))