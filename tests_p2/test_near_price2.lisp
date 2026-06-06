(define (run)
  (let* ((resp (http-get "https://api.rhea.finance/get-token-price?token_id=wrap.near"))
         (price (json-get-str "price" resp)))
    (str-cat "NEAR price: $" price)))
