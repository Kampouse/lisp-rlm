;; Test all u128 builtins together
(define (register)
  (if (near/deposit-gte 1000000000000000000)
    0
    (near/panic "min deposit")))

(near/transfer "kampouse.near" 1000000000000000000000000)

(export "register" register)
