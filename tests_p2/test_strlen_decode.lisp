;; Test str-len on decoded bytes
(define (run)
  (let* ((bytes "[72,101,108,108,111]")
         (decoded (json-decode-bytes bytes))
         (len (str-len decoded)))
    (str-cat "Decoded: " decoded " (len: " (to-string len) ")")))