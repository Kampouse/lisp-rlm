;; Test storage delete
(define (run)
  (let ((del (storage-delete "mykey")))
    (let ((get (storage-get "mykey")))
      get)))
