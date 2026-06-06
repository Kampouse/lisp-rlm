;; Test JSON extraction from Rhea prices
(define (run)
  (let* ((prices-url "https://api.rhea.finance/list-token-price")
         (prices-json (http-get prices-url))
         
         ;; Try different extraction paths
         (wrap-near (json-get-str "wrap.near" prices-json))
         (usdt-obj (json-get-str "usdt.tether-token.near" prices-json)))
    
    (str-cat "wrap.near: " wrap-near "\n"
             "usdt: " usdt-obj)))