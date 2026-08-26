(define (run)
  (let* (
    (args-b64 "eyJhY2NvdW50X2lkIjoia2FtcG91c2UubmVhciJ9")
    (rpc-body (str-cat "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"query\",\"params\":{\"request_type\":\"call_function\",\"finality\":\"final\",\"account_id\":\"contract.main.burrow.near\",\"method_name\":\"get_account\",\"args_base64\":\"" args-b64 "\"}}"))
    (rpc-result (http-post "https://rpc.mainnet.fastnear.com" rpc-body))
    )
    (str-cat "rpc len:" (to-string (str-len rpc-result)) " | " rpc-result)))