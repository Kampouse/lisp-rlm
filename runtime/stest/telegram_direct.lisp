;; Direct send-telegram test
(define (run)
  (let ((r "test message"))
    (outlayer/send-telegram "5125145880" r)))
(run)