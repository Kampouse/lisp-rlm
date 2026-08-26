;; Simple OutLayer test - storage
(define (run)
  (let ((result (outlayer/storage-set "test_key" "test_value")))
    (str-concat "{\"stored\":\"" result "\"}")))
