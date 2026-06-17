(define (run)
  (let* ((account_id ""))
    (json-get-wasi "account_id" "str" account_id)
    (str-cat "{\"account\":\"" account_id "\"}")))
