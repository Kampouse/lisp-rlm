;; Test: Decode bytes and extract value
(define (run)
  (let* ((bytes "[123,34,97,99,99,111,117,110,116,95,105,100,34,58,34,116,101,115,116,49,50,51,34,125]")
         (decoded (json-decode-bytes bytes))
         (account-id (json-get-str "account_id" decoded)))
    (str-cat "Decoded len: " (to-string (str-len decoded)) " | account_id: " account-id)))