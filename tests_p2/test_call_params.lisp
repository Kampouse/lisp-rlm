;; Test signed contract call with parameters
;; Call wrap.testnet ft_balance_of with account_id param
(define (run input)
  (let* (
    (signer-id (env/get "NEAR_ACCOUNT_ID"))
    (signer-key (env/get "NEAR_SIGNER_KEY"))
    ;; JSON args: {"account_id": "kampy.testnet"}
    (args "{\"account_id\":\"kampy.testnet\"}")
    (result (if (and signer-id signer-key)
      (near/call-signed signer-id signer-key "wrap.testnet" "ft_balance_of" args "0" "30000000000000" "FINAL")
      "missing env vars")))
    result))