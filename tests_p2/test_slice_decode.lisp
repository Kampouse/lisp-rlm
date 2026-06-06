;; Test: Decode bytes and check first few chars directly
(define (run)
  (let* ((bytes "[123, 34, 97, 34, 58, 34, 104, 101, 108, 108, 111, 34, 125]")
         (decoded (json-decode-bytes bytes))
         (first-char (str-slice decoded 0 5)))
    (begin
      (print (str-cat "Decoded len: " (to-string (str-len decoded))))
      (print (str-cat "First 5: " first-char))
      "Done")))