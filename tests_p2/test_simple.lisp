(define (run)
  ;; Simple test: return first 10 chars of JSON input using json-get-str
  (let* ((s (json-get-str "account_id" input)))
    (str-cat "{\"result\":\"" s "\"}")))