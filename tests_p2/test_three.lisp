;; Test: Three bytes
(define (run)
  (let* ((bytes "[65,66,67]")
         (decoded (json-decode-bytes bytes)))
    (str-cat "Len: " (to-string (str-len decoded)))))