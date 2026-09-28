;;;
;;; pool_v3_token — minimal NEP-141-style mock token for pool_v3 wasm
;;; tests (test-only companion, lives under pool-v3/ per task fences).
;;;
;;; Ledger: "b:<acct>" → u128 decimal string. Implements:
;;;   mint_to(account_id, amount)            test faucet
;;;   ft_transfer(receiver_id, amount)       requires 1 yocto attached
;;;   ft_transfer_call(receiver_id, amount, msg)
;;;       NEP-141 flow: debit sender, credit receiver, promise
;;;       ft_on_transfer on the receiver, then ft_resolve_transfer on
;;;       THIS contract reads the keep-string and refunds
;;;       amount − keep to the sender in the same callback.
;;;   ft_resolve_transfer(...)              internal callback
;;;   ft_balance_of(account_id) view        raw u128 string [g]
;;;
;;; msg is embedded into the promise args JSON escaped (str-replace
;;; '"' → '\"') so JSON msgs like {"min_out":"1"} survive the round trip.
;;;
(define TGAS 30000000000000)

(define (sget k d) (default (near/storage_get k) d))
(define (sput k v) (near/storage_set k v))

(define (fail m)
  (near/log m)
  (near/abort m))

(define (bal-of a) (sget (str-cat "b:" a) "0"))

(define (mint-to)
  (let ((acct (default (near/json_get_str "account_id") ""))
        (amt (default (near/json_get_str "amount") "0")))
    (let ((b (bal-of acct)))
      (let ((b2 (u128/add b amt)))
        (sput (str-cat "b:" acct) b2)
        (near/return b2)))))

(define (ft-transfer)
  ;; NOTE: the near-mock's promise_create host pins attached deposit to
  ;; 0 for promise-path calls, so the NEP-141 one-yocto guard is only
  ;; enforced on ROOT calls (ft_transfer_call below). Accept 0 or 1 here.
  (let ((to (default (near/json_get_str "receiver_id") ""))
        (amt (default (near/json_get_str "amount") "0"))
        (from (near/predecessor_account_id))
        (dep (near/attached_deposit_u128)))
    (if (and (not (= dep "1")) (not (= dep "0")))
        (fail "ERR_ONE_YOCTO")
        (let ((b (bal-of from)))
          (let ((b2 (u128/sub b amt)))
            (sput (str-cat "b:" from) b2)
            (let ((t (bal-of to)))
              (let ((t2 (u128/add t amt)))
                (sput (str-cat "b:" to) t2)
                (near/return "1"))))))))

(define (ft-transfer-call)
  (let ((to (default (near/json_get_str "receiver_id") ""))
        (amt (default (near/json_get_str "amount") "0"))
        (msg (default (near/json_get_str "msg") ""))
        (from (near/predecessor_account_id))
        (dep (near/attached_deposit_u128)))
    (if (not (= dep "1"))
        (fail "ERR_ONE_YOCTO")
        (let ((b (bal-of from)))
          (let ((b2 (u128/sub b amt)))
            (sput (str-cat "b:" from) b2)
            (let ((t (bal-of to)))
              (let ((t2 (u128/add t amt)))
                (sput (str-cat "b:" to) t2)
                ;; escrow done — promise the receiver's ft_on_transfer
                (let ((esc (str-replace msg "\"" "\\\"")))
                  (let ((args (str-cat "{\"sender_id\":\"" from
                                      "\",\"amount\":\"" amt
                                      "\",\"msg\":\"" esc "\"}")))
                    (let ((idx (near/promise_batch_create to)))
                      (near/promise_batch_action_function_call
                       idx "ft_on_transfer" args "1" TGAS)
                      (let ((self (near/current_account_id)))
                        (let ((cb (near/promise_batch_then idx self)))
                          (near/promise_batch_action_function_call
                           cb "ft_resolve_transfer"
                           (str-cat "{\"sender_id\":\"" from
                                    "\",\"receiver_id\":\"" to
                                    "\",\"amount\":\"" amt "\"}")
                           "0" TGAS)
                          (near/promise_return cb)))))))))))))

(define (ft-resolve-transfer)
  (let ((sender (default (near/json_get_str "sender_id") ""))
        (recv (default (near/json_get_str "receiver_id") ""))
        (amt (default (near/json_get_str "amount") "0")))
    (let ((ok (= (near/promise_succeeded 0) 1)))
      (let ((keep (if ok
                      (let ((r (near/promise_result 0)))
                        (if (u128/gt r amt) amt r))
                      "0")))
        (let ((refund (u128/sub amt keep)))
          (if (u128/eq refund "0")
              (near/return keep)
              (let ((t (bal-of recv)))
                (let ((t2 (u128/sub t refund)))
                  (sput (str-cat "b:" recv) t2)
                  (let ((s (bal-of sender)))
                    (let ((s2 (u128/add s refund)))
                      (sput (str-cat "b:" sender) s2)
                      (near/return keep)))))))))))

(define (ft-balance-of)
  (let ((acct (default (near/json_get_str "account_id") "")))
    (let ((b (bal-of acct)))
      (near/return b))))

(export "mint_to" mint-to)
(export "ft_transfer" ft-transfer)
(export "ft_transfer_call" ft-transfer-call)
(export "ft_resolve_transfer" ft-resolve-transfer)
(export "ft_balance_of" ft-balance-of #t)
