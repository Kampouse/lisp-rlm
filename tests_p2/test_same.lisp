;; Test: Single repeated byte
(define (run)
  (let* ((bytes "[65,65,65]")
         (decoded (json-decode-bytes bytes)))
    (str-cat "Len: " (to-string (str-len decoded)))))