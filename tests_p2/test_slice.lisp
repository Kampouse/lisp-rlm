(define (test)
  (let ((inp "hello"))
    (let ((len (bytes-to-u32 (str-slice inp 0 4))))
      (near/return_str (to-string len)))))
