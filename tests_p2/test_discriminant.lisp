(define (run)
  ;; Debug: check discriminant at RET_AREA after blocking_read
  ;; If non-zero, there's an error
  (let* ((disc (near/i64 126976)))
    (str-cat "{\"discriminant\":" (to-string disc) "}")))