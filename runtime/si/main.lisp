;; Verify json-get-str extracts clean content — no json-sanitize needed
(define (run input)
  (let ((raw (http-post "https://api.z.ai/api/coding/paas/v4/chat/completions"
    "{\"model\":\"glm-5-turbo\",\"max_tokens\":100,\"thinking\":{\"type\":\"enabled\"},\"messages\":[{\"role\":\"user\",\"content\":\"Say OK\"}]}")))
    (let ((content (json-get-str "choices.message.content" raw)))
      (if (nil? content) "FAIL-nil"
        (str-concat "OK: " content)))))
