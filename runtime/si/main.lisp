;; Simulate multi-phase-agent fetch-1 exactly
(define (run input)
  (let ((r (http-post "https://httpbin.org/post"
    "{\"data\":\"NEAR DeFi protocol data source 1\"}")))
    (storage-set "task:data-1" (json-sanitize r))
    (storage-set "task:phase" "fetch-2")
    "fetch-1-done"))
