;; Test storage roundtrip: store then read back
;; This proves outlayer/storage-set and outlayer/storage-get work end-to-end
(define (run)
  (let* (
    (result (outlayer/storage-set "test_key" "hello_storage"))
    (read (outlayer/storage-get "test_key"))
    )
    read))
