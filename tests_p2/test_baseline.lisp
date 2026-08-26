;; Simplest possible HTTP GET test
(define (run)
  (http-get "https://api.rhea.finance/list-token-price"))
