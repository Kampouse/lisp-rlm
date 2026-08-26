;;; standing-intent.lisp — Persistent AI Agent with Standing Intent
;;;
;;; Telegram-accessible: bot sends {"text":"msg"} via stdin,
;;; agent calls AI and replies via send-telegram.
;;;
;;; Storage keys (for serve/tick mode):
;;;   "inbox:latest"   - user message
;;;   "inbox:pending"  - "1" = unprocessed, "0" = idle
;;;   "agent:intent"   - standing intent type
;;;
;;; Two modes:
;;;   run(input) — bot mode: input from stdin, reply via send-telegram
;;;   tick()      — serve mode: inbox from storage, reply via send-telegram

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
    "{\"model\":\"glm-5-turbo\",\"max_tokens\":500,"
    "\"messages\":[{\"role\":\"user\",\"content\":\""
    user-msg
    "\"}]}")))
    (http-post "https://api.z.ai/api/coding/paas/v4/chat/completions" body)))

;; === Extract content from ZAI response ===

(define (extract-content resp)
  (let ((choices (json-get "choices" resp)))
    (if (nil? choices) resp
      (let ((msg (json-get "message" choices)))
        (if (nil? msg) resp
          (let ((content (json-get "content" msg)))
            (if (nil? content) resp content)))))))

;; === Dispatch ===

(define (tick)
  (let ((pending (has-pending?)))
    (if (= pending 1)
      (let ((msg (storage-get "inbox:latest")))
        (if (= msg "")
          (begin (clear-inbox) "no-message")
          (begin
            (send-telegram "5125145880" (extract-content (call-ai msg)))
            (clear-inbox)
            "responded")))
      (if (= (storage-get "agent:intent") "")
        (begin
          (storage-set "agent:intent" "telegram-ai-assistant")
          "booted")
        "idle"))))

;; === Run Entry Point (bot mode: input from stdin) ===

(define (run input)
  (let ((text (json-get "text" input)))
    (if (nil? text) "idle"
      (send-telegram "5125145880" (extract-content (call-ai text))))))
