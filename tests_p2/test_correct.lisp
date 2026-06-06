;; Test: Decode correct JSON - {"a":"hi"} = [123,34,97,34,58,34,104,105,34,125]
(define (run)
  (let* ((bytes "[123,34,97,34,58,34,104,105,34,125]")
         (decoded (json-decode-bytes bytes))
         (len (str-len decoded)))
    (str-cat "Decoded " (to-string len) " bytes: " decoded)))