;; Test: Single byte
(define (run)
  (let* ((bytes "[65]")
         (decoded (json-decode-bytes bytes)))
    (str-cat "Len: " (to-string (str-len decoded)))))