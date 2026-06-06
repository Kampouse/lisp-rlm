;; Burrow Account Summary - Direct output test
(define (run)
  (let* ((near-json (http-get "https://api.rhea.finance/get-token-price?token_id=wrap.near"))
         (usdt-json (http-get "https://api.rhea.finance/get-token-price?token_id=usdt.tether-token.near"))
         (near-price (json-get-str "price" near-json))
         (usdt-price (json-get-str "price" usdt-json)))
    (str-cat "NEAR: $" near-price " | USDT: $" usdt-price)))