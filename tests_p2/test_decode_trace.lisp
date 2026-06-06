;; Test: Check actual byte array length
(define (run)
  (let* ((bytes "[123,34,97,99,99,111,117,110,116,95,105,100,34,58,34,116,101,115,116,49,50,51,34,125]")
         (byte-len (str-len bytes))
         (decoded (json-decode-bytes bytes))
         (dec-len (str-len decoded)))
    (str-cat "Byte array len: " (to-string byte-len) " | Decoded len: " (to-string dec-len))))