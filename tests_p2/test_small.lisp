(define (run)
  (let* ((resp (http-get "https://api.rhea.finance/list-token-price"))
         (len (str-len resp)))
    (str-cat "Got " (str-cat (int-to-str len) " bytes"))))
