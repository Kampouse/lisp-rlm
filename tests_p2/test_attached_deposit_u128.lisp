(define (check)
  ;; Test near/attached_deposit_u128
  (let ((dep (near/attached_deposit_u128)))
    dep))
(export "check" check)
