;;; multi-phase-agent.lisp — Complex Multi-Step Standing Intent Agent
;;;
;;; Multi-tick state machine: idle → fetch-1 → fetch-2 → analyze → deliver → idle
;;;
;;; Uses NEAR RPC for data fetching (clean JSON, no control chars).
;;; json-sanitize applied when building AI prompt from stored data.

;; === Helpers ===

(define (has-pending?)
  (let ((p (storage-get "inbox:pending")))
    (if (nil? p) nil
      (if (= p "1") 1 0))))

(define (clear-inbox)
  (begin
    (storage-set "inbox:latest" "")
    (storage-set "inbox:pending" "0")))

(define (call-ai prompt)
  (let ((body (str-concat
    "{\"model\":\"glm-5-turbo\",\"max_tokens\":4096,\"thinking\":{\"type\":\"enabled\"},"
    "\"messages\":[{\"role\":\"user\",\"content\":\""
    (json-sanitize prompt)
    "\"}]}")))
    (http-post "https://api.z.ai/api/coding/paas/v4/chat/completions" body)))

;; === Phase Handlers ===

(define (handle-idle)
  (let ((pending (has-pending?)))
    (if (nil? pending) "idle"
      (if (= pending 1)
        (begin
          (storage-set "task:prompt" (storage-get "inbox:latest"))
          (clear-inbox)
          (storage-set "task:phase" "fetch-1")
          "task-started")
        "idle"))))

(define (handle-fetch-1)
  ;; Fetch NEAR gas price via RPC
  (let ((r (http-post "https://rpc.testnet.near.org"
    "{\"jsonrpc\":\"2.0\",\"id\":\"fetch1\",\"method\":\"gas_price\",\"params\":[]}")))
    (storage-set "task:data-1" r)
    (storage-set "task:phase" "fetch-2")
    "fetch-1-done"))

(define (handle-fetch-2)
  ;; Fetch NEAR status via RPC
  (let ((r (http-post "https://rpc.testnet.near.org"
    "{\"jsonrpc\":\"2.0\",\"id\":\"fetch2\",\"method\":\"status\",\"params\":[]}")))
    (storage-set "task:data-2" r)
    (storage-set "task:phase" "analyze")
    "fetch-2-done"))

(define (handle-analyze)
  (let ((d1 (storage-get "task:data-1")))
    (let ((d2 (storage-get "task:data-2")))
      (let ((data (str-concat "Gas price data: " (json-sanitize d1) " | Network status: " (json-sanitize d2))))
        (let ((prompt (str-concat
          "You are a NEAR Protocol analyst. Analyze this data in 2-3 sentences: "
          data)))
          (let ((analysis (call-ai prompt)))
            (storage-set "task:analysis" analysis)
            (storage-set "task:phase" "deliver")
            "analyzed"))))))

(define (handle-deliver)
  (let ((analysis (storage-get "task:analysis")))
    (outlayer/send-telegram "5125145880" analysis)
    (storage-set "task:phase" "idle")
    "delivered"))

;; === Tick Dispatch ===

(define (tick)
  (let ((phase (storage-get "task:phase")))
    (if (nil? phase)
      (begin
        (storage-set "agent:intent" "multi-phase-agent")
        (storage-set "task:phase" "idle")
        "booted")
      (if (= phase "idle") (handle-idle)
        (if (= phase "fetch-1") (handle-fetch-1)
          (if (= phase "fetch-2") (handle-fetch-2)
            (if (= phase "analyze") (handle-analyze)
              (if (= phase "deliver") (handle-deliver)
                "unknown-phase"))))))))

;; === Entry Point ===

(define (run input)
  (tick))
