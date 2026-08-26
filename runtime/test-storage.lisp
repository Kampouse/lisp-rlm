;; test-storage.lisp - minimal storage test
(define (run input)
  (storage-set "test" "hello")
  (let ((val (storage-get "test")))
    val))