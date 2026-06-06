(define (run)
  (let* ((resp (http-get "https://api.coingecko.com/api/v3/simple/price?ids=near&vs_currencies=usd"))
         (len (str-len resp)))
    (str-cat "Got " (str-cat (int-to-str len) " bytes: " resp))))
