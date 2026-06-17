(define (run input)
  (let* ((acct (json-get-str "account_id" input)))
    (str-cat "{\"account\":\"" acct "\"}")))