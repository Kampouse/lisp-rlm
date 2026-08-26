;; Burrow Tracker — permissionless margin position tracking service.
;; Users deposit NEAR to fund tracking. Cron calls poll() → OutLayer WASM.
;;
;; Storage keys (using near/kstore for user-specific keys):
;;   "owner"         — account_id of contract owner
;;   "poll/{owner}"  — "1" if account authorized to poll
;;   "{account}/deposit"  — u128 user deposit (via near/kstore)
;;   "{account}/credits"   — i64 tracking credits remaining

(define (ky acct key) key) ;; near/kstore handles key concatenation internally

(define (init)
  (begin
    (near/store-bytes "owner" (near/signer_account_id))
    (near/kstore "poll" (near/signer_account_id) 1)
    (near/return_str "ok")))

(define (register)
  (if (near/deposit-gte 1000000000000000000)
    (let ((acct (near/signer_account_id)))
      (begin
        (near/store_u128 (str-slice acct 0 (strlen acct)) acct (near/attached_deposit_u128))
        (near/kstore acct "credits" 1000)
        (near/return_str "registered")))
    (near/panic "minimum 0.001 NEAR")))

(define (withdraw)
  (let ((acct (near/signer_account_id)))
    (if (near/kload acct "deposit")
      (let ((dep (near/load_u128 acct)))
        (begin
          (near/kremove acct "deposit")
          (near/kremove acct "credits")
          (near/transfer acct dep)
          (near/return_str "withdrawn")))
      (near/panic "no deposit"))))

(define (poll)
  (if (near/kload "poll" (near/signer_account_id))
    (near/return_str "polled")
    (near/panic "only owner can poll")))

(define (get_balance)
  (let ((inp (near/input)))
    (let ((acct-len (bytes-to-u32 (str-slice inp 0 4))))
      (let ((acct (str-slice inp 4 (+ 4 acct-len))))
        (if (near/kload acct "deposit")
          (let ((dep (near/load_u128 acct)))
            (let ((buf-addr (+ dep 128)))
              (near/return_str (u128/to_str dep buf-addr))))
          (near/return_str "0"))))))

(define (get_info)
  (near/return_str (near/load-bytes "owner")))

(export "init" init false)
(export "register" register false)
(export "withdraw" withdraw false)
(export "poll" poll false)
(export "get_balance" get_balance true)
(export "get_info" get_info true)