;; Burrow Tracker — permissionless margin position tracking service.
;; Users deposit NEAR to fund tracking. Cron calls poll() → OutLayer WASM.
;;
;; Storage keys:
;;   "owner"               — account_id of contract owner
;;   "poll/{owner}"        — "1" if account authorized to poll
;;   "{account}/deposit"   — u128 user deposit (16 bytes via store_u128)
;;   "{account}/credits"   — i64 tracking credits remaining

(define (init)
  (near/store-bytes "owner" (near/signer_account_id))
  (near/store-bytes (str-cat "poll/" (near/signer_account_id)) "1")
  (near/return_str "ok"))

(define (register)
  (if (near/deposit-gte 1000000000000000000)
    (let ((acct (near/signer_account_id)))
      (near/store_u128 (str-cat acct "/deposit") (near/attached_deposit_u128))
      (near/store (str-cat acct "/credits") 1000)
      (near/return_str "registered"))
    (near/panic "minimum 0.001 NEAR")))

(define (withdraw)
  (let ((acct (near/signer_account_id)))
    (if (near/has_key (str-cat acct "/deposit"))
      (let ((dep (near/load_u128 (str-cat acct "/deposit"))))
        (near/remove (str-cat acct "/deposit"))
        (near/remove (str-cat acct "/credits"))
        (near/transfer acct dep)
        (near/return_str "withdrawn"))
      (near/panic "no deposit"))))

(define (poll)
  (if (near/has_key (str-cat "poll/" (near/signer_account_id)))
    (near/return_str "polled")
    (near/panic "only owner can poll")))

(define (get_balance)
  (let ((acct (near/predecessor_account_id)))
    (near/return_str (str-concat
      (to-string (near/load_u128 (str-cat acct "/deposit"))) 
      ","
      (to-string (near/load (str-cat acct "/credits")))))))

(define (get_info)
  (near/return_str (near/load-bytes "owner")))

(export "init" init)
(export "register" register)
(export "withdraw" withdraw)
(export "poll" poll)
(export "get_balance" get_balance)
(export "get_info" get_info)