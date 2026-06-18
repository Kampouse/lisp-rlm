;;; standing-intent.lisp — Persistent AI Agent with Standing Intent
;;;
;;; Standing intent: "telegram-ai-assistant"
;;;   Perpetual agent: reads user message from storage, calls AI,
;;;   sends response to Telegram. idles when no work pending.
;;;
;;; Storage keys:
;;;   "inbox:latest"   - user message (set externally by Hermes/SQLite)
;;;   "inbox:pending"  - "1" = unprocessed, "0" = idle
;;;   "agent:intent"   - standing intent type (set once at boot)

;; === Storage Helpers ===

(define (has-pending?)
  (let ((p (storage-get "inbox:pending")))
    (if (nil? p) nil
      (if (= p "1") 1 0))))

(define (clear-inbox)
  (begin
    (storage-set "inbox:latest" "")
    (storage-set "inbox:pending" "0")))

;; === AI Call ===
;; Builds JSON body dynamically from user message in storage

(define (call-ai user-msg)
  (let ((body (str-concat
    "{\"model\":\"glm-5-turbo\",\"max_tokens\":4096,\"thinking\":{\"type\":\"enabled\"},"
    "\"messages\":[{\"role\":\"user\",\"content\":\""
    user-msg
    "\"}]}")))
    (http-post "https://api.z.ai/api/coding/paas/v4/chat/completions" body)))

;; === Dispatch ===

(define (tick)
  (let ((pending (has-pending?)))
    (if (nil? pending)
      ;; First boot — register standing intent, go idle
      (begin
        (storage-set "agent:intent" "telegram-ai-assistant")
        "booted")
      (if (= pending 1)
        ;; Work to do — read message, call AI, respond via Telegram
        (let ((msg (storage-get "inbox:latest")))
          (if (nil? msg)
            ;; Race: flag set but message cleared
            (begin (clear-inbox) "no-message")
            (begin
              (outlayer/send-telegram "5125145880" (call-ai msg))
              (clear-inbox)
              "responded")))
        ;; Idle — nothing to do
        "idle"))))

;; === Run Entry Point ===

(define (run input)
  (tick))
