(define (run input)
  (let* ((acct (json-get-str "account_id" input)))
    (str-cat "{\"len\":" (to-string (str-len acct)) ",\"ptr\":" (to-string (str-ptr acct)) "}")))