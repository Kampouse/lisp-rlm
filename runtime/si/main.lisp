;;; standing-intent.lisp — Persistent AI Agent with Standing Intent
;;;
;;; Standing intent: "telegram-ai-assistant"
;;;   Perpetual agent: ticks check for new messages, calls AI, responds via Telegram
;;;
;;; Storage keys:
;;;   "inbox:latest"   - user message (set externally by Hermes/SQLite)
;;;   "inbox:pending"  - "1" = unprocessed, "0" = idle
;;;   "agent:intent"   - standing intent type (set once at boot)
;;;
;;; Flow:
;;;   1. Hermes injects message + sets pending=1 in SQLite
;;;   2. inlayer serve ticks WASM → tick()
;;;   3. tick() → AI call → send-telegram → clears inbox
;;;   4. Next tick → idle (17ms return)

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
;; NOTE: str-concat with large literals + storage result causes memory faults
;; So we store the pre-built body externally and let the host fill it in.
;; For now: call AI with a fixed prompt. Standing intent decides WHAT to ask.
;; TODO: when str-concat memory fault is fixed, inject user-msg dynamically

(define (call-ai)
  (http-post "https://api.z.ai/api/coding/paas/v4/chat/completions"
    "{\"model\":\"glm-5-turbo\",\"max_tokens\":4096,\"thinking\":{\"type\":\"enabled\"},\"messages\":[{\"role\":\"user\",\"content\":\"What is the current state of NEAR Protocol in one paragraph?\"}]}"))

;; === Dispatch ===

(define (tick)
  (let ((pending (has-pending?)))
    (if (nil? pending)
      ;; First boot — register standing intent, go idle
      (begin
        (storage-set "agent:intent" "telegram-ai-assistant")
        "booted")
      (if (= pending 1)
        ;; Work to do — call AI, respond via Telegram
        (begin
          (outlayer/send-telegram "5125145880" (call-ai))
          (clear-inbox)
          "responded")
        ;; Idle — nothing to do
        "idle"))))

;; === Run Entry Point ===

(define (run input)
  (tick))
