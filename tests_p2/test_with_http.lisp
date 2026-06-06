(define (run)
  (let* (
    ;; Dummy HTTP call to initialize runtime
    (_ (http-get "https://api.rhea.finance/list-token-price"))
    (account-id (json-get-str "account_id"))
    )
    (str-cat "{\"len\":" (to-string (str-len account-id)) "}")))