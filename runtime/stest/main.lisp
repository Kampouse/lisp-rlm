;; AI + Telegram harness (max_tokens:4096, no content extraction yet)
;; TODO: extract choices[0].message.content once json-get works in P2
(define (run)
  (let ((body "{\"model\":\"glm-5-turbo\",\"max_tokens\":4096,\"thinking\":{\"type\":\"enabled\"},\"messages\":[{\"role\":\"user\",\"content\":\"Say hello in exactly one sentence.\"}]}"))
    (let ((r (http-post "https://api.z.ai/api/coding/paas/v4/chat/completions" body)))
      (outlayer/send-telegram "5125145880" r)
      r)))
(run)
