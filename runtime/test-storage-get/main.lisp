(define (run input)
  (if (= input "")
    "empty"
    (begin
      (storage-set "test-key" "hello")
      (let ((val (storage-get "test-key")))
        (if (nil? val)
          "no val"
          val)))))
