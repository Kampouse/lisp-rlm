;; Test storage increment
(define (run)
  (let ((v1 (storage-increment "counter" 1)))
    (let ((v2 (storage-increment "counter" 5)))
      v2)))
