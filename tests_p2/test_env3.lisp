;; Test env/get - reads NEAR_ACCOUNT_ID from config
(define (run input)
  (let* (
    (account (env/get "NEAR_ACCOUNT_ID"))
    (result (if account account "not_set"))
    (out (str-cat "{\"account\":\"" result "\"}"))
    )
    out))