(define (run input)
  (let* ((raw (json-get-str "account_id" input)))
    (str-cat "{\"len\":" (to-string (str-len raw)) "}")))