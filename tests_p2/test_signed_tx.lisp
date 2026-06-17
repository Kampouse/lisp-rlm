;; Test signed transaction with explicit key
(define (run input)
  (let* (
    (signer-id (env/get "NEAR_ACCOUNT_ID"))
    (signer-key (env/get "NEAR_SIGNER_KEY"))
    (result (if (and signer-id signer-key)
      (near/call-signed 
        signer-id
        signer-key
        signer-id
        "ping"
        "{}"
        "0"
        "30000000000000"
        "FINAL")
      "missing")))
    result))