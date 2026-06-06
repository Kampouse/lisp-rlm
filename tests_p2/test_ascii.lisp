;; Test: Decode simpler ASCII bytes
(define (run)
  (let* ((bytes "[65,66,67,68,69]")
         (decoded (json-decode-bytes bytes)))
    (str-cat "Len: " (to-string (str-len decoded)))))