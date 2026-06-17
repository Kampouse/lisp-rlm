(define (run)
  (let* ((account-id (json-get-str "account_id" input)))
    (str-cat "{\"received\":\"" account-id "\",\"len\":" (to-string (str-len account-id)) "}")))
