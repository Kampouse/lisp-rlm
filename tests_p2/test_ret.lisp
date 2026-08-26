(define (run)
  ;; Test: Check RET_AREA after blocking_read
  ;; discriminant at 126976, ptr at 126980, len at 126984
  (let* ((disc (near/i64 126976))
         (ptr (near/i64 126980))
         (len (near/i64 126984)))
    (str-cat "{\"disc\":" (to-string disc) 
              ",\"ptr\":" (to-string ptr) 
              ",\"len\":" (to-string len) "}")))