(define (run)
  (let ((resp (http-get "https://api.rhea.finance/list-token-price")))
    resp))
