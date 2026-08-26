(define (test)
  (let ((x (near/kload "c/" "test.near")))
    (near/return_str (to-string x))))
