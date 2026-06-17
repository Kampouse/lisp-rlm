;; Test signed transfer - send 1 yoctoNEAR to yourself
(define (run input)
  (let* (
    (signer-id (env/get "NEAR_ACCOUNT_ID"))
    (signer-key (env/get "NEAR_SIGNER_KEY"))
    (result (if (and signer-id signer-key)
      (near/transfer-signed signer-id signer-key signer-id "1" "FINAL")
      "missing env vars")))
    result))