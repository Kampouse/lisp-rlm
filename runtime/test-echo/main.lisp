(define (run input)
  (begin
    (storage-set "test:key" "hello-value")
    (let ((data (storage-get "test:key")))
      (if (nil? data) "still-no-data"
        (str-concat "data:" data)))))
