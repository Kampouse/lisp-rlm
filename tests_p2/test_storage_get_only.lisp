;; Test: just storage-get for the key we just set
(define (run)
  (outlayer/storage-get "test_key"))
