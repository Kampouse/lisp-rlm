(define (run input)
  (let* ((acct (json-get-str "account_id" input)))
    (str-cat "{\"received\":\"" acct "\",\"len\":" (to-string (str-len acct)) "}")))
