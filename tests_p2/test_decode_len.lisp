;; Simple test - just decode and show length
(define (run)
  (let* ((bytes "[72,101,108,108,111]")
         (decoded (json-decode-bytes bytes))
         (len (str-len decoded)))
    (str-cat "Decoded: " decoded " (len: " len ")")))