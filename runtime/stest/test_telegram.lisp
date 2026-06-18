;; Simple send-telegram test
(define (run)
  (outlayer/send-telegram "5125145880" "Test from harness.wasm")
  "done")
(run)