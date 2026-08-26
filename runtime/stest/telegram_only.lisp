;; Minimal send-telegram test - no other calls
(define (run)
  (outlayer/send-telegram "5125145880" "Direct test from WASM")
  "done")