;; Test: http-get (known working) then check for corruption
(define (run)
  (http-get "https://rpc.mainnet.near.org" "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"query\",\"params\":{\"request_type\":\"view_account\",\"finality\":\"final\",\"account_id\":\"outlayer.near\"}}"))
