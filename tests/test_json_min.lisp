(memory 4)

(define (add_amount)
  (let ((a (near/json_get_int "amount")))
    (near/log_num a)
    (+ a 1)))

(define (foo) 42)

(export "add_amount" add_amount true)
(export "foo" foo true)
