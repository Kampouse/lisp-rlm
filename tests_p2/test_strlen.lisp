;; Test str-len on known string
(define (run)
  (let* ((s "Hello, World!")
         (len (str-len s)))
    (str-cat "Len: " (to-string len))))