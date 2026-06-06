;; Test decode only
;; bytes = [123,34,97,34,58,49,125] = a:1 object
(define (run)
  (let* ((bytes "[123, 34, 97, 34, 58, 49, 125]")
         (decoded (json-decode-bytes bytes)))
    (str-cat "Decoded len: " (to-string (str-len decoded)))))