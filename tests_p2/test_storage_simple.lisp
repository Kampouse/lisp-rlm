;; Minimal storage test
(define (run input)
  (let ((s1 (storage-set "testkey" "testval")))
    (let ((s2 (storage-get "testkey")))
      s2)))
