;; Test: Decode bytes without spaces
(define (run)
  (let* ((bytes "[72,101,108,108,111]")
         (decoded (json-decode-bytes bytes)))
    (to-string (str-len decoded))))