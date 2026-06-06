(define (run)
  (let* ((prices (http-get "https://api.rhea.finance/list-token-price"))
         (near-data (json-get-str "wrap.near" prices)))
    near-data))
