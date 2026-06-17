;; Burrow Tracker v6 — minimal, working.
;; Deposit NEAR for margin tracking credits.
;; Returns deposit as yocto string (simplified, no u128/to_str).
;;
;; Keys:
;;   "owner" — contract owner
;;   "poll/" + account — poll auth (near/kstore)
;;   "u/" + account — u128 deposit (near/store_u128)
;;   "c/" + account — credits (near/kstore)

(define (init)
  (begin
    (near/store-bytes "owner" (near/signer_account_id))
    (near/kstore "poll/" (near/signer_account_id) 1)
    (near/return_str "ok")))

(define (register)
  (if (near/deposit-gte 1000000000000000000)
    (let ((acct (near/signer_account_id)))
      (begin
        (near/store_u128 (str-cat "u/" acct) (near/attached_deposit_u128))
        (near/kstore "c/" acct 1000)
        (near/return_str "registered")))
    (near/panic "minimum 0.001 NEAR")))

(define (withdraw)
  (let ((acct (near/signer_account_id)))
    (if (near/has_key (str-cat "u/" acct))
      (let ((dep-ptr (near/load_u128 (str-cat "u/" acct))))
        (begin
          (near/remove (str-cat "u/" acct))
          (near/remove (str-cat "c/" acct))
          (near/transfer acct dep-ptr)
          (near/return_str "withdrawn")))
      (near/panic "no deposit"))))

(define (poll)
  (if (near/kload "poll/" (near/signer_account_id))
    (near/return_str "polled")
    (near/panic "only owner can poll")))

;; Returns "lo,hi" format (two u64 values representing u128)
(define (get_balance)
  (let ((inp (near/input)))
    (let ((acct-len (bytes-to-u32 (str-slice inp 0 4))))
      (let ((acct (str-slice inp 4 (+ 4 acct-len))))
        (if (near/has_key (str-cat "u/" acct))
          (let ((dep-ptr (near/load_u128 (str-cat "u/" acct))))
            (near/return_str (str-cat
              (to-string (u128/load dep-ptr))
              ","
              (to-string (u128/load_high dep-ptr)))))
          (near/return_str "0,0"))))))

(define (get_credits)
  (let ((inp (near/input)))
    (let ((acct-len (bytes-to-u32 (str-slice inp 0 4))))
      (let ((acct (str-slice inp 4 (+ 4 acct-len))))
        (near/return_str (to-string (near/kload "c/" acct)))))))

(define (get_info)
  (near/return_str (near/load-bytes "owner")))

(export "init" init false)
(export "register" register false)
(export "withdraw" withdraw false)
(export "poll" poll false)
(export "get_balance" get_balance true)
(export "get_credits" get_credits true)
(export "get_info" get_info true)