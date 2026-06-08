;; Test outlayer/transfer compilation — 4 args: signer_id signer_key receiver amount
;; This version uses empty strings for test (host will error, which proves the call path works)
(define (run)
  (let* (
    (tx (outlayer/transfer "test.near" "test_key" "kampouse.near" "1"))
    )
    tx))
