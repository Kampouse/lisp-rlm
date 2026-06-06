(define (run)
  (str-len (http-get "https://api.rhea.finance/list-token-price")))
