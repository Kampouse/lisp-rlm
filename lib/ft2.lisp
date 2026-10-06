;; lib/ft2.lisp — minimal NEP-141 token with the full ft_transfer_call
;; promise dance (2026-10-06). One wasm, deployed per-token (separate
;; accounts = separate storage). Storage: "own" → owner, "b:<acct>" →
;; decimal-string balance, "sup" → supply.
;; Entries: new · mint · ft_balance_of (view) · ft_transfer ·
;;          ft_transfer_call → promise→ receiver.ft_on_transfer →
;;          callback ft_resolve_transfer re-credits the refund string.
;;
;; NOTE: f-transfer/f-transfer-call use SEQUENTIAL single-binding lets —
;; the wasm emitter has a local-slot collision in multi-binding lets with
;; heap-allocating inits (seen on-chain 2026-10-06: bal/amt slots aliased,
;; li-cmp read garbage → "insufficient" with a valid balance).

(define (num-ok? s)
  ;; <=40 digits, ASCII only — OOB-parse guard
  (if (> (str-length s) 40)
      "0"
      (loop ((i 0))
        (if (>= i (str-length s))
            "1"
            (let ((d (- (byte-at s i) 48)))
              (if (or (< d 0) (> d 9))
                  "0"
                  (recur (+ i 1))))))))

(define (bz k) (let ((v (near/load-bytes k))) (if (nil? v) "0" v)))
(define (bkey who) (str-cat "b:" who))
(define (bal who) (bz (bkey who)))
(define (add-bal who d) (near/store-bytes (bkey who) (li-add (bal who) d)))
(define (sub-bal who d) (near/store-bytes (bkey who) (li-sub (bal who) d)))

(define (f-new)
  (begin (near/store-bytes "own" (near/predecessor_account_id)) (near/return "ok")))

(define (f-mint)
  (let ((inp (near/input)))
    (if (str= (near/predecessor_account_id) (bz "own"))
        (let ((to (json-get-str "to" inp)))
          (let ((amt (json-get-str "amt" inp)))
            (if (str= (num-ok? amt) "0")
                (near/return "bad-amt")
                (if (> (li-cmp (li-add (bz "sup") amt) "1000000") 0)
                    (near/return "cap")
                    (begin
                      (add-bal to amt)
                      (near/store-bytes "sup" (li-add (bz "sup") amt))
                      (near/return "ok"))))))
        (near/return "not-owner"))))

(define (f-balance)
  (near/return (bal (json-get-str "account_id" (near/input)))))

;; NEP-141 core: sender = predecessor. {"receiver_id","amount"}
(define (f-transfer)
  (let ((inp (near/input)))
    (let ((who (near/predecessor_account_id)))
      (let ((to (json-get-str "receiver_id" inp)))
        (let ((amt (json-get-str "amount" inp)))
          (if (str= (num-ok? amt) "0")
              (near/return "bad-amt")
          (begin
            (if (< (li-cmp (bal who) amt) 0)
                (near/return "insufficient")
                (begin
                  (near/log (str-cat (str-cat "@ft_transfer from:" who) (str-cat " to:" to)))
                  (sub-bal who amt) (add-bal to amt) (near/return "ok"))))))))))

;; NEP-141 ft_transfer_call: debit, promise→receiver.ft_on_transfer,
;; callback here gets the receiver's refund string via promise_result(0).
;; {"receiver_id","amount","msg"} → promise chain result = "used:<n>"
(define (f-transfer-call)
  (let ((inp (near/input)))
    (let ((sender (near/predecessor_account_id)))
      (let ((recv (json-get-str "receiver_id" inp)))
        (let ((amt (json-get-str "amount" inp)))
          (if (str= (num-ok? amt) "0")
              (near/return "bad-amt")
          (if (< (li-cmp (bal sender) amt) 0)
              (near/return "insufficient")
              (begin
                (sub-bal sender amt)
                (add-bal recv amt)
                (let ((p (near/promise_batch_create recv)))
                  (begin
                    (near/promise_batch_action_function_call p "ft_on_transfer"
                      (json-set (json-set (json-set "{}" "sender_id" sender) "amount" amt)
                                "msg" (json-get-str "msg" inp))
                      "0" 100000000000000)
                    (near/promise_return
                      (near/promise_then p (near/current_account_id) "ft_resolve_transfer"
                        (json-set (json-set (json-set "{}" "sender_id" sender) "receiver_id" recv) "amount" amt)
                        "0" 30000000000000))))))))))))

;; callback: {"sender_id","receiver_id","amount"} — receiver was credited
;; upfront; move the refund slice back (empty result = failed → full refund).
(define (f-resolve)
  (let ((inp (near/input)))
    (let ((sender (json-get-str "sender_id" inp)))
      (let ((recv (json-get-str "receiver_id" inp)))
        (let ((amt (json-get-str "amount" inp)))
          (let ((res (near/promise_result 0)))
            (let ((refund (if (< (str-length res) 1) amt res)))
              (begin (add-bal sender refund)
                     (sub-bal recv refund)
                     (near/return (str-cat "used:" (li-sub amt refund)))))))))))

(export "new" f-new)
(export "mint" f-mint)
(export "ft_balance_of" f-balance true)
(export "ft_transfer" f-transfer)
(export "ft_transfer_call" f-transfer-call)
(export "ft_resolve_transfer" f-resolve)
