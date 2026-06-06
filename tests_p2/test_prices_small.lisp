;; Test with smaller per-token endpoint
(define (run)
  (let* ((near-price-url "https://api.rhea.finance/get-token-price?token_id=wrap.near")
         (usdt-price-url "https://api.rhea.finance/get-token-price?token_id=usdt.tether-token.near")
         
         (near-json (http-get near-price-url))
         (usdt-json (http-get usdt-price-url))
         
         (near-price (json-get-str "price" near-json))
         (usdt-price (json-get-str "price" usdt-json)))
    
    (str-cat "NEAR: $" near-price "\n"
             "USDT: $" usdt-price)))