(define (run)
  (let* (
    ;; Real base64 for {"account_id":"943addabde7913c6f58043d348ec763643689b68da2e7ab186b7d75e1d544ff2"}
    (args-b64 "eyJhY2NvdW50X2lkIjoiOTQzYWRkYWJkZTc5M2M2ZjU4MDQzZDM0OGVjNzYzNjQzNjg5YjY4ZGEyZTdhYjE4NmI3ZDc1ZTFkNTQ0ZmYyMiJ9")
    (rpc-body (str-cat "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"query\",\"params\":{\"request_type\":\"call_function\",\"finality\":\"final\",\"account_id\":\"contract.main.burrow.near\",\"method_name\":\"get_account\",\"args_base64\":\"" args-b64 "\"}}"))
    (rpc-result (http-post "https://rpc.mainnet.fastnear.com" rpc-body))
    )
    (str-cat "len:" (to-string (str-len rpc-result)) " | " rpc-result)))