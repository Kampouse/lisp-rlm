
;; ── NEAR method shims (deploy surface — appended by gen_deploy.py) ──
;; Identity inside the contract is signer-based (corpus convention);
;; shims only unpack args. Amounts: u128 decimal strings. "" = failure.
(define (m-init) (near/json_return_str (init)))

(define (m-propose)
  (near/json_return_str
    (propose (near/json_get_str "id") (near/json_get_str "recipient")
             (near/json_get_str "amount"))))

(define (m-approve)
  (near/json_return_str
    (approve (near/json_get_str "id") (near/json_get_str "recipient"))))

(define (m-execute)
  (near/json_return_str
    (execute (near/json_get_str "id") (near/json_get_str "recipient"))))

(define (m-cancel)
  (near/json_return_str
    (cancel (near/json_get_str "id") (near/json_get_str "recipient"))))

(define (m-tx-amount)
  (near/json_return_str (tx-amount (near/json_get_str "id"))))

(define (m-approvals)
  (near/json_return_str (u128/from-i64 (approvals (near/json_get_str "id")))))

(export "init" m-init false)
(export "propose" m-propose false)
(export "approve" m-approve false)
(export "execute" m-execute false)
(export "cancel" m-cancel false)
(export "tx_amount" m-tx-amount true)
(export "approvals" m-approvals true)
