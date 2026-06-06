;; Test: Check if values are stored correctly
(define (run)
  (let* ((bytes "[65,66,67]")
         (decoded (json-decode-bytes bytes))
         (len (str-len decoded)))
    (begin
      (print (str-cat "Output len: " (to-string len)))
      "Done")))