;; Test: First check if json-get-str works on decoded buffer
(define (run)
  (let* ((bytes "[123,34,97,99,99,111,117,110,116,95,105,100,34,58,34,116,101,115,116,49,50,51,34,125]")
         (decoded (json-decode-bytes bytes))
         (val (json-get-str "account_id" decoded)))
    (str-cat "Val: '" val "'")))