(define (run)
  (let* ((acct (json-get-str "account_id" input)))
    (str-cat "account_id: [" acct "] len=" (to-string (str-len acct)))))
