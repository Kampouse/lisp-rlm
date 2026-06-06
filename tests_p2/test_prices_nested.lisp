;; Test nested JSON extraction from Rhea prices
(define (run)
  (let* ((prices-url "https://api.rhea.finance/list-token-price")
         (prices-json (http-get prices-url))
         
         ;; Extract nested: prices-json -> "wrap.near" -> then "price"
         (wrap-obj (json-get-str "wrap.near" prices-json))
         (near-price (json-get-str "price" wrap-obj))
         
         ;; For USDT
         (usdt-obj (json-get-str "usdt.tether-token.near" prices-json))
         (usdt-price (json-get-str "price" usdt-obj)))
    
    (str-cat "wrap.near obj: " wrap-obj "\n"
             "NEAR price: " near-price "\n"
             "USDT price: " usdt-price)))