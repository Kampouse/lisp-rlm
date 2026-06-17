;; Burrow Tracker v8 — Fixed for current lisp-rlm compiler
;; Uses near/kstore and near/kload for string-prefixed keys (FP_GLOBAL-safe)
;;
;; API:
;;   init() → "ok"
;;   register() → "registered" (attaches deposit as credits)
;;   get_credits() → credits count (simplified - just returns fixed value)

(define (init)
  (begin
    (near/kstore "owner" "" 1)
    (near/kstore "poll/" (near/signer_account_id) 1)
    (near/return_str "ok")))

(define (register)
  (let ((acct (near/signer_account_id)))
    (begin
      (near/kstore "c/" acct (near/attached_deposit))
      (near/return_str "registered"))))

(define (get_credits)
  (near/return_str "1000"))

(define (poll)
  (if (near/kload "poll/" (near/signer_account_id))
    (near/return_str "polled")
    (near/panic "only owner can poll")))

(define (get_info)
  (near/return_str "burrow_tracker_v8"))

(export "init" init false)
(export "register" register false)
(export "get_credits" get_credits true)
(export "poll" poll false)
(export "get_info" get_info true)