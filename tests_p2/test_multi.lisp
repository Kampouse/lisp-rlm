;; Test: Multi-digit values  
(define (run)
  (let* ((bytes "[72,101]")
         (decoded (json-decode-bytes bytes)))
    (str-cat "Len: " (to-string (str-len decoded)))))