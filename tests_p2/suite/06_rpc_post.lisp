(define (run)
  (http-post "https://rpc.mainnet.fastnear.com" 
    "{\"jsonrpc\":\"2.0\",\"method\":\"query\",\"params\":[\"account/0\",{\"account_id\":\"kampouse.near\"}],\"id\":1}" 
    "{\"Content-Type\": \"application/json\"}"))
