(define (run input)
  (let ((val (storage-get "test")))
    (if val
      "found"
      "not found")))
