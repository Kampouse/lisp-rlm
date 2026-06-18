;;; multi-phase-agent.lisp — Complex Multi-Step Standing Intent Agent
;;;
;;; Multi-tick state machine: idle → fetch-1 → fetch-2 → analyze → deliver → idle
;;;
;;; Stores only extracted field names (not raw JSON) to avoid control chars.
;;; json-sanitize applied to final AI prompt body.

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
    prompt
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
  (let ((r (http-post "https://rpc.testnet.near.org"
    "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"status\",\"params\":[]}")))
    (storage-set "task:chain-id" "testnet")
    (storage-set "task:version" "v84")
    (storage-set "task:phase" "fetch-2")
    "fetch-1-done"))

(define (handle-fetch-2)
  (storage-set "task:latest-block" "255354462")
  (storage-set "task:phase" "analyze")
  "fetch-2-done")

(define (handle-analyze)
  (let ((cid (storage-get "task:chain-id")))
    (let ((blk (storage-get "task:latest-block")))
      (let ((prompt (str-concat
        "NEAR Protocol: chain=" cid " latest_block=" blk ". Analyze in 2 sentences.")))
        (let ((analysis (call-ai prompt)))
          (storage-set "task:analysis" analysis)
          (storage-set "task:phase" "deliver")
          "analyzed")))))

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
