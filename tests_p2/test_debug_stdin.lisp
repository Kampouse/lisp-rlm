;; Diagnostic: print ptr and len from blocking-read
(define (run input)
  (let* ((acct (json-get-str "account_id" input)))
    acct))