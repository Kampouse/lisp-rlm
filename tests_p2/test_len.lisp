;; Test: Decode and check first chars - no json-get
(define (run)
  (let* ((bytes "[123,34,97,34,58,49,50,51,34,125]")
         (decoded (json-decode-bytes bytes))
         (len (str-len decoded)))
    (str-cat "Decoded " (to-string len) " bytes")))