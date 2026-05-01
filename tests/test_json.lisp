(memory 4)

(define (add_amount)
  (let ((a (near/json_get_int "amount")))
    (near/log_num a)
    (+ a 1)))

(define (greet)
  (let ((name (near/json_get_str "name")))
    (near/log name)))

(define (return_int)
  (let ((val (near/json_get_int "value")))
    (near/json_return_int (+ val 10))))

(export "add_amount" add_amount true)
(export "greet" greet true)
(export "return_int" return_int true)
