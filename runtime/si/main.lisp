(define (run input)
  (let ((raw (http-post "https://rpc.testnet.near.org" "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"status\",\"params\":[]}")))
    (json-get-str "result.chain_id" raw)))
