;; mock_ft: minimal NEP-141 for scenario work — the phase-3 probe contract.
;; STATIC FIXTURE (not generator-emitted): exists to answer by execution:
;;   P1  two-hop delivery: ft_transfer_call -> promise(attached) ->
;;       receiver ft_on_transfer with {amount, msg} — does msg/attach arrive?
;;   P2  abort rollback: receiver aborts mid-callback — is receiver storage
;;       rolled back to pre-call state?
;;   P3  resolve-refund: after abort, ft_resolve observes failure
;;       (promise_result "") and re-credits sender (ledger bounce).
;; Ledger is storage fiction (house style — no value-transfer primitive):
;;   s:reg:<acct> "1"  registered   s:bal:<acct> u128 balance
(define SELF "ft.clmm.test.near")
(define TGAS 10000000000000)

(define (sput k v) (near/storage_set k v))
(define (sget k d) (default (near/storage_get k) d))
(define (fail m) (near/log m) (near/abort m))
(define (add a b) (u128/add a b))
(define (sub a b) (u128/sub a b))
(define (ult a b) (u128/lt a b))
(define (bal a) (sget (str-cat "s:bal:" a) "0"))
(define (credit a v) (sput (str-cat "s:bal:" a) (add (bal a) v)))
(define (debit a v)
  (if (ult (bal a) v) (fail "ERR_BAL") "ok")
  (sput (str-cat "s:bal:" a) (sub (bal a) v)))

;; seed supply (scenario setup): owner mints to an account
(define (mint)
  (let* ((to (default (near/json_get_str "to") ""))
         (v (default (near/json_get_str "amount") "0")))
    (begin (credit to v) (near/return (bal to)))))

;; NEP-141 ft_transfer_call: debit sender NOW, promise amount+msg into
;; receiver's ft_on_transfer with the amount attached, then resolve.
(define (ft_transfer_call)
  (let* ((sender (near/predecessor_account_id))
         (rid (default (near/json_get_str "receiver_id") ""))
         (v (default (near/json_get_str "amount") "0"))
         (msg (default (near/json_get_str "msg") "")))
    (begin
      (debit sender v)
      ;; args JSON for the receiver: {"amount": V, "msg": "..."}
      ;; (str-cat is BINARY in this dialect — nested pairs, house style)
      (let ((args (str-cat "{\"amount\":\""
                    (str-cat v (str-cat "\",\"msg\":\"" (str-cat msg "\"}"))))))
        (let ((p (near/promise_create rid "ft_on_transfer" args v TGAS)))
          (near/promise_then p SELF "ft_resolve"
                             (str-cat "{\"sid\":\""
                               (str-cat sender (str-cat "\",\"v\":\"" (str-cat v "\"}"))))
                             "0" TGAS)
          (near/return "pending"))))))

;; resolve: promise kept  -> receiver handled it, nothing to do.
;;         promise failed -> refund sender (the bounce, observed).
(define (ft_resolve)
  (let* ((r (near/promise_result 0))
         (sid (default (near/json_get_str "sid") ""))
         (v (default (near/json_get_str "v") "0")))
    (if (= r "")
        (begin (credit sid v)
               (near/log (str-cat "refund sid=" (str-cat sid (str-cat " v=" v))))
               (near/return "refunded"))
        (near/return "kept"))))

;; plain transfer (no callback) for scenario asserts
(define (ft_transfer)
  (let* ((sender (near/predecessor_account_id))
         (rid (default (near/json_get_str "to") ""))
         (v (default (near/json_get_str "amount") "0")))
    (begin (debit sender v) (credit rid v) (near/return "ok"))))

(define (ft_balance_of)
  (near/return (bal (default (near/json_get_str "account") ""))))

(define (register)
  (let ((a (default (near/json_get_str "account_id") "")))
    (begin (sput (str-cat "s:reg:" a) "1") (near/return "ok"))))

(export "mint" mint)
(export "ft_transfer" ft_transfer)
(export "ft_transfer_call" ft_transfer_call)
(export "ft_resolve" ft_resolve)
(export "ft_balance_of" ft_balance_of)
(export "register" register)
