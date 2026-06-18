;; Test json-get-str with AI response dot-path on http-post result
(define (run input)
  (let ((raw (http-post "https://api.z.ai/api/coding/paas/v4/chat/completions"
    "{\"model\":\"glm-5-turbo\",\"max_tokens\":200,\"thinking\":{\"type\":\"enabled\"},\"messages\":[{\"role\":\"user\",\"content\":\"Say hello in one word\"}]}")))
    (let ((content (json-get-str "choices.message.content" raw)))
      (if (nil? content) "NOT-FOUND"
        (str-concat "AI: " content)))))
