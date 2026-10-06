;; lib/clmm_pool_v4.lisp — book-ownership LP pool over the v2 ladder.
;; Model: LPs deposit B (token-B via ft_on_transfer, predecessor == TOKB).
;; First deposit bootstraps the ladder (fixed shape 1000:2000:5000:3000:1500,
;; later deposits proportional to current slot fill). Swaps (token-A side,
;; predecessor == TOKA, msg "swap") walk the ladder as v2; ALL A received
;; (amt - rem) accrues to "AB" — LP revenue. Withdraw burns shares, pays out
;; BOTH sides: out_A = sh·AB//SHT (promise on TOKA) chained into a callback
;; that pays out_B = sh·PB//SHT (promise on TOKB). Slots shrink by the
;; withdrawn B fraction → sellable-B ≤ pool-B invariant preserved (floored).
;; Storage: S0..S4 (A-capacity, start ZERO), F (fee display, A), TOKA/TOKB,
;; AB (A book), PB (B book ≡ B balance), SHT, SH:<a>.
;; Share price (A+B per share) rises with swap volume — fees included in AB.

(define (bz k) (let ((v (near/load-bytes k))) (if (nil? v) "0" v)))
(define (sh-key a) (str-cat "SH:" a))

(define (self-call?) (str= (near/predecessor_account_id) (near/current_account_id)))

(define (num-ok? s)
  ;; <=40 digits, ASCII only — OOB-parse & u128-scale guard (Cetus-class)
  (if (> (str-length s) 40)
      "0"
      (loop ((i 0))
        (if (>= i (str-length s))
            "1"
            (let ((d (- (byte-at s i) 48)))
              (if (or (< d 0) (> d 9))
                  "0"
                  (recur (+ i 1))))))))

(define (pool-init4)
  ;; one-shot: a bound TOKA means already initialized — state untouched
  (if (nil? (near/load-bytes "TOKA"))
      (begin
        (slot-set 0 "0") (slot-set 1 "0") (slot-set 2 "0")
        (slot-set 3 "0") (slot-set 4 "0")
        (near/store-bytes "F" "0")
        (near/store-bytes "AB" "0")
        (near/store-bytes "PB" "0")
        (near/store-bytes "SHT" "0")
        (near/store-bytes "TOKA" (json-get-str "toka" (near/input)))
        (near/store-bytes "TOKB" (json-get-str "tokb" (near/input)))
        "ok")
      "already-initialized"))

(define (slot-sum)
  (li-add (li-add (li-add (li-add (slot-get 0) (slot-get 1)) (slot-get 2)) (slot-get 3)) (slot-get 4)))

;; slot i += (amt·base_i//tot)·100//pnum_i — base = shape (first) or slot (later)
(define (slot-widen i base amt tot pnum)
  (slot-set i (li-add (slot-get i)
    (u128-muldiv (u128-muldiv amt base tot) "100" pnum))))

(define (add-b-liq amt)
  ;; returns the unused-B refund for ft_on_transfer: "0" on success.
  ;; Validate BEFORE mutating: dust / zero-share / bad-amount all refund untouched.
  (if (< (li-cmp amt "10") 0)
      amt  ;; below min deposit (10): refund, no state write (sybil/DoS guard)
      (let ((sh (if (str= (bz "SHT") "0")
                amt
                (u128-muldiv amt (bz "SHT") (bz "PB")))))
    (if (str= sh "0")
        amt  ;; full refund — dust deposit floors to zero shares
        (let ((tot (slot-sum)))
          (begin
            (if (str= tot "0")
                (begin (slot-widen 0 "1000" amt "12500" "98")
                       (slot-widen 1 "2000" amt "12500" "99")
                       (slot-widen 2 "5000" amt "12500" "100")
                       (slot-widen 3 "3000" amt "12500" "101")
                       (slot-widen 4 "1500" amt "12500" "102"))
                (begin (slot-widen 0 (slot-get 0) amt tot "98")
                       (slot-widen 1 (slot-get 1) amt tot "99")
                       (slot-widen 2 (slot-get 2) amt tot "100")
                       (slot-widen 3 (slot-get 3) amt tot "101")
                       (slot-widen 4 (slot-get 4) amt tot "102")))
            (near/store-bytes "SHT" (li-add (bz "SHT") sh))
            (near/store-bytes "PB" (li-add (bz "PB") amt))
            (near/store-bytes (sh-key (json-get-str "sender_id" (near/input)))
              (li-add (bz (sh-key (json-get-str "sender_id" (near/input)))) sh))
            "0"))))))

(define (pay-out)
  ;; internal only — returns the swap refund value to the token resolve
  (if (self-call?)
      (near/return (json-get-str "r" (near/input)))
      (near/return "forbidden")))

(define (pay-b)
  ;; callback leg 2 of withdrawal: pay out_B to r — internal only
  (if (self-call?)
      (let ((inp (near/input)))
        (let ((p (near/promise_batch_create (near/load-bytes "TOKB"))))
          (begin
            (near/promise_batch_action_function_call p "ft_transfer"
              (json-set (json-set "{}" "receiver_id" (json-get-str "r" inp)) "amount" (json-get-str "b" inp))
              "0" 40000000000000)
            (near/promise_return p))))
      (near/return "forbidden")))

(define (withdraw4)
  (let ((inp (near/input)))
    (let ((who (near/predecessor_account_id)))
      (let ((sh (json-get-str "sh" inp)))
        (if (str= (num-ok? sh) "0")
            "bad-sh"
        (if (< (li-cmp (bz (sh-key who)) sh) 0)
            (str-cat "insufficient-shares-have:" (bz (sh-key who)))
            (let ((ab (bz "AB")) (pb (bz "PB")) (sht (bz "SHT")))
              (let ((out-a (u128-muldiv sh ab sht))
                    (out-b (u128-muldiv sh pb sht)))
                (begin
                  (near/store-bytes "SHT" (li-sub sht sh))
                  (near/store-bytes (sh-key who) (li-sub (bz (sh-key who)) sh))
                  (near/store-bytes "AB" (li-sub ab out-a))
                  (near/store-bytes "PB" (li-sub pb out-b))
                  ;; shrink slots by withdrawn-B fraction (floor = safe side)
                  (let ((old-pb (li-add (bz "PB") out-b)))
                    (begin
                      (slot-set 0 (u128-muldiv (slot-get 0) (bz "PB") old-pb))
                      (slot-set 1 (u128-muldiv (slot-get 1) (bz "PB") old-pb))
                      (slot-set 2 (u128-muldiv (slot-get 2) (bz "PB") old-pb))
                      (slot-set 3 (u128-muldiv (slot-get 3) (bz "PB") old-pb))
                      (slot-set 4 (u128-muldiv (slot-get 4) (bz "PB") old-pb))))
                  (if (str= out-a "0")
                      (let ((p2 (near/promise_batch_create (near/load-bytes "TOKB"))))
                        (begin
                          (near/promise_batch_action_function_call p2 "ft_transfer"
                            (json-set (json-set "{}" "receiver_id" who) "amount" out-b)
                            "0" 40000000000000)
                          (near/promise_return p2)
                          "ok-b"))
                      (let ((p (near/promise_batch_create (near/load-bytes "TOKA"))))
                        (begin
                          (near/promise_batch_action_function_call p "ft_transfer"
                            (json-set (json-set "{}" "receiver_id" who) "amount" out-a)
                            "0" 40000000000000)
                          (near/promise_return
                            (near/promise_then p (near/current_account_id) "pay_b"
                              (json-set (json-set "{}" "r" who) "b" out-b)
                              "0" 60000000000000))
                          "ok-ab"))))))))))))

(define (msg-min msg)
  ;; "swap" → "0"; "swap:NNN" → NNN
  (if (< (str-length msg) 6)
      "0"
      (str-substring msg 5 (str-length msg))))

(define (ft-on-transfer4)
  (let ((inp (near/input)))
    (let ((sender (json-get-str "sender_id" inp)))
      (let ((amt (json-get-str "amount" inp)))
        (if (str= (num-ok? amt) "0")
            (near/return amt)
            (if (str= (near/predecessor_account_id) (near/load-bytes "TOKB"))
            (near/return (add-b-liq amt))
            (if (str= (near/predecessor_account_id) (near/load-bytes "TOKA"))
                (let ((min (msg-min (json-get-str "msg" inp))))
                  (if (str= (num-ok? min) "0")
                      (near/return amt)
                  (let ((f0 (fee-get))
                        (s0v (slot-get 0)) (s1v (slot-get 1)) (s2v (slot-get 2))
                        (s3v (slot-get 3)) (s4v (slot-get 4)))
                    (pool-swap amt)
                    (let ((dy (bz "DY")) (rem (bz "REM")))
                      (if (< (li-cmp dy min) 0)
                          ;; slippage guard: restore pre-walk state, refund all A
                          (begin
                            (slot-set 0 s0v) (slot-set 1 s1v) (slot-set 2 s2v)
                            (slot-set 3 s3v) (slot-set 4 s4v)
                            (near/store-bytes "F" f0)
                            (near/return amt))
                          (begin
                            (near/store-bytes "AB" (li-add (bz "AB") (li-sub amt rem)))
                            (near/store-bytes "PB" (li-sub (bz "PB") dy))
                            (if (str= dy "0")
                                (near/return rem)
                                (let ((p (near/promise_batch_create (near/load-bytes "TOKB"))))
                                  (begin
                                    (near/promise_batch_action_function_call p "ft_transfer"
                                      (json-set (json-set "{}" "receiver_id" sender) "amount" dy)
                                      "0" 40000000000000)
                                    (near/promise_return
                                      (near/promise_then p (near/current_account_id) "pay_out"
                                        (json-set "{}" "r" rem) "0" 5000000000000)))))))))))
                (near/return amt))))))))

(export "ft_on_transfer" ft-on-transfer4)
(export "pay_out" pay-out)
(export "pay_b" pay-b)
