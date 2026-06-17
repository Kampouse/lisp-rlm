;; Test env/get and contract call (P1 mode)
;; Usage: NEAR_ACCOUNT_ID=kampy.testnet inlayer run test_env_p1.wasm '{}' --target=wasi-p1

(define (run input)
  (let* (
    ;; Read account from env var (set via inlayer config or CLI)
    (env-account (env/get "NEAR_ACCOUNT_ID"))
    (acct (if env-account env-account "test.testnet"))
    
    ;; Test contract call using the account
    (args (str-cat "{\"account_id\":\"" acct "\"}"))
    (result (outlayer/view "v2.proposal.burrow.testnet" "get_account" args))
    
    ;; Return both env var and result
    (out (str-cat "{\"env_account\":\"" acct 
                  "\",\"has_result\":" (if (nil? result) "false" "true")
                  ",\"result\":\"" (if (nil? result) "null" result) "\"}"))
    )
    out))