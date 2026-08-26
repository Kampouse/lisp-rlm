(define (run)
  (let* (
    (_ (http-get "https://api.rhea.finance/list-token-price"))
    )
    "{\"status\":\"http-ok\"}"))