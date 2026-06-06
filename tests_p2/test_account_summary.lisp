;; Burrow Account Summary - fetches account and prices
;; Output: Account data with USD values
(define (run)
  (let* ((account "943addabde7913c6f58043d348ec763643689b68da2e7ab186b7d75e1d544ff2")
         (args-b64 "eyJhY2NvdW50X2lkIjoiOTQzYWRkYWJkZTc5MTNjNmY1ODA0M2QzNDhlYzc2MzY0MzY4OWI2OGRhMmU3YWIxODZiN2Q3NWUxZDU0NGZmMiJ9")
         
         ;; Fetch prices (per-token endpoint - smaller response)
         (near-json (http-get "https://api.rhea.finance/get-token-price?token_id=wrap.near"))
         (usdt-json (http-get "https://api.rhea.finance/get-token-price?token_id=usdt.tether-token.near"))
         
         (near-price (json-get-str "price" near-json))
         (usdt-price (json-get-str "price" usdt-json))
         
         ;; Fetch Burrow account
         (rpc (http-post "https://rpc.mainnet.near.org"
                         (str-cat "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"query\",\"params\":{\"request_type\":\"call_function\",\"finality\":\"final\",\"account_id\":\"contract.main.burrow.near\",\"method_name\":\"get_account\",\"args_base64\":\"" args-b64 "\"}}")))
         
         ;; Extract arrays from result
         (result-bytes (json-get-str "result" rpc))
         (supplied-obj (json-get-str "supplied" result-bytes)))
    
    (str-cat "=== BURROW ACCOUNT ===\n"
             "Account: " account "\n"
             "NEAR: $" near-price "\n"
             "USDT: $" usdt-price "\n"
             "Supplied: " supplied-obj)))