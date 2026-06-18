;; Test storage roundtrip: set then get, verify persistence
(define (run input)
  (let* (
    (set-result (storage-set "hello" "world"))
    (get-result (storage-get "hello"))
    ;; Also test with complex key
    (set-result2 (storage-set "counter" "42"))
    (get-result2 (storage-get "counter")))
    ;; Return a JSON-like response showing both results
    (outlayer/str-cat
      "{\"set1\":\"" (or set-result "nil")
      "\",\"get1\":\"" (or get-result "nil")
      "\",\"set2\":\"" (or set-result2 "nil")
      "\",\"get2\":\"" (or get-result2 "nil")
      "\"}")))
