;; Test env/get with two variables
(define (run input)
  (let* (
    (account (env/get "NEAR_ACCOUNT_ID"))
    (key (env/get "NEAR_SIGNER_KEY"))
    (result (if (and account key)
      (str-cat "Account: " account " Key: " (str-slice key 0 20) "...")
      "Missing vars")))
    result))