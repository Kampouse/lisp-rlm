(define (init)
  (begin
    (near/kstore "poll/" "test.near" 1)
    (near/return_str "ok")))
