;; Test storage functions
(define (run input)
  (let ((data (storage-get "test-key")))
    (if (nil? data)
      (begin
        (storage-set "test-key" "hello")
        "stored")
      data)))

;; Expected: "stored" first run, "hello" second run