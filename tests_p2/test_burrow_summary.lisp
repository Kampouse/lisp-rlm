;; Burrow Account Summary - fetches account data and token prices
;; Displays: Supplied, Collateral, Borrowed with USD values
(define (run)
  (let* ((account "943addabde7913c6f58043d348ec763643689b68da2e7ab186b7d75e1d544ff2")
         (args-b64 "eyJhY2NvdW50X2lkIjoiOTQzYWRkYWJkZTc5MTNjNmY1ODA0M2QzNDhlYzc2MzY0MzY4OWI2OGRhMmU3YWIxODZiN2Q3NWUxZDU0NGZmMiJ9")
         (prices-url "https://api.rhea.finance/list-token-price")
         
         ;; Fetch account data
         (rpc (http-post "https://rpc.mainnet.near.org"
                         (str-cat "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"query\",\"params\":{\"request_type\":\"call_function\",\"finality\":\"final\",\"account_id\":\"contract.main.burrow.near\",\"method_name\":\"get_account\",\"args_base64\":\"" args-b64 "\"}}")))
         
         ;; Fetch token prices  
         (prices-json (http-get prices-url))
         
         ;; Extract prices (json-get-str returns empty if not found)
         (near-price (json-get-str "wrap.near.price" prices-json))
         (usdt-price (json-get-str "usdt.tether-token.near.price" prices-json))
         
         ;; Extract first supplied token
         (supplied-raw (json-get-str "supplied" rpc)))
    
    (str-cat "Account: " account "\n"
             "NEAR: $" near-price "\n"
             "USDT: $" usdt-price "\n"
             "Supplied: " supplied-raw)))