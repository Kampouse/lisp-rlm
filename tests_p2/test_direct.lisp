(define (run)
  ;; Test: Check what's at RET_AREA after stdin read
  ;; Just return the string from json-get-str directly to verify stdin is populated
  (let* ((s (json-get-str "account_id" input)))
    (str-cat "{\"result\":\"" s "\",\"len\":" (to-string (str-len s)) "}")))