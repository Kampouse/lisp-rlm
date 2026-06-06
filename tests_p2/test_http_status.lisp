;; Test HTTP GET with str-len
(define (run)
  (let* ((near-json (http-get "https://api.rhea.finance/get-token-price?token_id=wrap.near"))
         (len (str-len near-json))
         (price (json-get-str "price" near-json)))
    (str-cat "Status: OK\nLen: " len "\nPrice: " price)))