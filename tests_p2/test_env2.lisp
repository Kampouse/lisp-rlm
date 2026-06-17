;; Test env/get with NEAR_NETWORK_ID (set in ~/.inlayer/config.toml)
(define (run input)
  (let* (
    (network (env/get "NEAR_NETWORK_ID"))
    (acct (if network network "not_set"))
    (out (str-cat "{\"network\":\"" acct "\"}"))
    )
    out))