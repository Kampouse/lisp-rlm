;; Test print lengths
(define (run)
  (let* ((s "Hello, World!"))
    (begin
      (print (str-cat "Len: " (to-string (str-len s))))
      "OK")))