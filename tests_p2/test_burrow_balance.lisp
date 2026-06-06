(define (run)
  (let* ((rpc (http-post "https://archival-rpc.mainnet.near.org"
                         "{"jsonrpc":"2.0","id":"dontcare","method":"query","params":{"request_type":"view_account","finality":"final","account_id":"b0b9fd60d4be5866.lisp1.c0d3f2.m0"}}"))
         (result (json-get-str "result" rpc))
         (balance (json-get-str "amount" result)))
    balance))
