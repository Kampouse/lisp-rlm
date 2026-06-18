;;; standing-intent.lisp — Persistent AI Agent with Standing Intent
;;;
;;; Standing intent: "telegram-ai-assistant"
;;;   Perpetual agent: reads user message from storage, calls AI,
;;;   sends response to Telegram. Idles when no work pending.
;;;
;;; Storage keys:
;;;   "inbox:latest"   - user message (set externally by Hermes/SQLite)
;;;   "inbox:pending"  - "1" = unprocessed, "0" = idle
;;;   "agent:intent"   - standing intent type (set once at boot)
;;;
;;; Flow:
;;;   1. Hermes injects message + sets pending=1 in SQLite
;;;   2. inlayer serve ticks WASM -> tick()
;;;   3. tick() reads message -> str-concat into JSON body -> AI call
;;;   4. AI response -> send-telegram -> clear inbox -> "responded"
;;;   5. Next tick -> idle (14ms)

;; === Storage Helpers ===

(define (has-pending?)
  (let ((p (storage-get "inbox:pending")))
    (if (= p "1") 1 0)))

(define (clear-inbox)
  (begin
    (storage-set "inbox:latest" "")
    (storage-set "inbox:pending" "0")))

;; === AI Call ===

(define (call-ai user-msg)
  (let ((body (str-concat
    "{\"model\":\"glm-5-turbo\",\"max_tokens\":4096,\"thinking\":{\"type\":\"enabled\"},"
    "\"messages\":[{\"role\":\"user\",\"content\":\""
    user-msg
    "\"}]}\"")))
    (http-post "https://api.z.ai/api/coding/paas/v4/chat/completions" body)))

;; === Dispatch ===

(define (tick)
  (let ((pending (has-pending?)))
    (if (= pending 1)
      ;; Work to do — read message, call AI, respond via Telegram
      (let ((msg (storage-get "inbox:latest")))
        (if (= msg "")
          ;; Race: flag set but message cleared
          (begin (clear-inbox) "no-message")
          (begin
            (outlayer/send-telegram "5125145880" (call-ai msg))
            (clear-inbox)
            "responded")))
      ;; Idle or first boot — check if intent registered
      (if (= (storage-get "agent:intent") "")
        (begin
          (storage-set "agent:intent" "telegram-ai-assistant")
          "booted")
        "idle"))))

;; === Run Entry Point ===

(define (run input)
  (tick))
