;; Debug: Show RET_AREA values after blocking-read
;; Tests what canonical ABI returns

(define (run)
  (let* ((_ (host-call 0 (i64-const 0)))          ;; get_stdin
         (handle (i32-load (i32-const 0)))        ;; handle from stack
         (_ (host-call 1 handle (i64-const 1000) (i32-const 126976)))  ;; blocking_read
         (disc (i32-load (i32-const 126976)))    ;; Result discriminant at RET_AREA[0]
         (ptr (i32-load (i32-const 126980)))      ;; ptr at RET_AREA[4]
         (len (i32-load (i32-const 126984)))       ;; len at RET_AREA[8]
         (_ (host-call 2 handle))                 ;; drop_input_stream
         (out (str-cat "{\"disc\":" (to-string disc)
                      ",\"ptr\":" (to-string ptr)
                      ",\"len\":" (to-string len) "}")))
    out))