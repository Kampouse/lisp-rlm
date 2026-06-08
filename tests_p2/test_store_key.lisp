;; Step 1: Store the signer key in OutLayer storage
;; Run this first, then run test_transfer_with_storage.lisp
(define (run)
  (outlayer/storage-set "signer_key" "ed25519:YOUR_KEY_HERE"))
