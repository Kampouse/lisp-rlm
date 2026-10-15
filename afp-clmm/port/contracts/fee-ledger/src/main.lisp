;; fee-ledger.lisp — User_Ledger.thy port (AFP afp-clmm, GREEN Oct 15 2026)
;;
;; The 4-jar fee machine, verbatim: per-user jars bal/paid/stranded +
;; checkpoint ck against aggregate G (2^64 fixed point), total SHT/shares,
;; global fees. Conservation theorem (holds for ALL runs, strand included):
;;   book(a) = bal + paid + pending + stranded  ≤  fees + grows   (from scratch)
;; Widths: u128 domain end-to-end (native limb-pair ops); TWO64 is a
;; string constant; pending = muldiv (G-ck) sh TWO64 — 128-bit intermediate
;; holds while G·SHT < 2^128, the same premise width_safety_run assumes.
;;
;; Topology (the theorem's demand): commit = PARK. withdraw moves the whole
;; credit (bal + owed) into stranded BEFORE the payout promise; the wd-cb
;; receipt delivers stranded → paid only in the success branch; a failed
;; receipt leaves the credit parked and visible — recover with strand-pay
;; (the theorem's StrandPay). WdFail = identity needs no code: a failed
;; receipt commits nothing.
;;
;; Mock simplification: the payout leg is a self-call receipt (no token
;; contract wired). On mainnet replace the batch target with the NEP-141
;; ft_transfer — the parking discipline is identical.
;;
;; Registration gate (USERS assumption → enforced): deposit/claim/withdraw
;; require reg:<a> = "1" or existing shares.

(define TWO64 "18446744073709551616")

(define (sput k v) (near/storage_set k v))
(define (sget k d) (default (near/storage_get k) d))
(define (fail m) (begin (near/log m) (near/abort m)))
(define (add a b) (u128/add a b))
(define (sub a b) (u128/sub a b))
(define (mdiv a b c) (u128/muldiv a b c))
(define (ult a b) (u128/lt a b))
(define (eq0 a) (u128/eq a "0"))

;; settle_inc (thy L68-71): guards agg<ck ∨ SHT=0 → 0
(define (settle who)
  (let* ((g (sget "G" "0"))
         (ck (sget (str-cat "ck:" who) "0"))
         (sht (sget "SHT" "0"))
         (sh (sget (str-cat "sh:" who) "0")))
    (if (ult g ck) "0"
        (if (eq0 sht) "0"
            (mdiv (sub g ck) sh TWO64)))))

(define (registered? who)
  (if (str= (sget (str-cat "reg:" who) "0") "1") "1"
      (if (eq0 (sget (str-cat "sh:" who) "0")) "0" "1")))

;; ── Grow f (admin): fees += f ONLY with attached = f (no minting credit
;;    from nothing); agg += f*TWO64/SHT (SHT=0 guard); grows counter +1 ──
(define (grow)
  (let* ((f (default (near/json_get_str "f") "0"))
         (att (near/attached_deposit_u128))
         (sht (sget "SHT" "0"))
         (g (sget "G" "0")))
    (if (eq0 f) (fail "ERR_F")
        (if (u128/eq att f) (begin
          (sput "G" (add g (if (eq0 sht) "0" (mdiv f TWO64 sht))))
          (sput "fees" (add (sget "fees" "0") f))
          (sput "grows" (add (sget "grows" "0") "1"))
          (near/log (str-cat "grow f=" f))
          (near/return "ok"))
            (fail "ERR_PAY")))))

;; ── register: the USERS gate ──
(define (register)
  (let ((who (near/predecessor_account_id)))
    (begin
      (sput (str-cat "reg:" who) "1")
      (near/log (str-cat "reg " who))
      (near/return "ok"))))

;; ── Deposit m (attach): settle-then-ckpt, mint shares ──
(define (deposit)
  (let* ((who (near/predecessor_account_id))
         (m (near/attached_deposit_u128)))
    (if (eq0 m) (fail "ERR_ZERO")
        (if (str= (registered? who) "0") (fail "ERR_REG")
            (let* ((owed (settle who))
                   (bk (str-cat "bal:" who))
                   (ckk (str-cat "ck:" who))
                   (shk (str-cat "sh:" who)))
              (begin
                (sput bk (add (sget bk "0") owed))
                (sput ckk (sget "G" "0"))
                (sput shk (add (sget shk "0") m))
                (sput "SHT" (add (sget "SHT" "0") m))
                (near/log (str-cat "deposit m=" m))
                (near/return "ok")))))))

;; ── Claim: settle into bal, ck := G ──
(define (claim)
  (let* ((who (near/predecessor_account_id)))
    (if (str= (registered? who) "0") (fail "ERR_REG")
        (let* ((owed (settle who))
               (bk (str-cat "bal:" who)))
          (begin
            (sput bk (add (sget bk "0") owed))
            (sput (str-cat "ck:" who) (sget "G" "0"))
            (near/log (str-cat "claim owed=" owed))
            (near/return owed))))))

;; ── withdraw n shares: burn + PARK whole credit (bal+owed) → stranded,
;;    then fire wd-cb receipt. Commit precedes delivery — the theorem. ──
(define (withdraw)
  (let* ((who (near/predecessor_account_id))
         (n (default (near/json_get_str "sh") "0"))
         (shk (str-cat "sh:" who)))
    (if (str= (registered? who) "0") (fail "ERR_REG")
        (if (ult (sget shk "0") n) (fail "ERR_SH")
            (let* ((owed (settle who))
                   (bk (str-cat "bal:" who))
                   (bnew (add (sget bk "0") owed))
                   (stk (str-cat "str:" who)))
              (begin
                (sput bk "0")
                (sput (str-cat "ck:" who) (sget "G" "0"))
                (sput shk (sub (sget shk "0") n))
                (sput "SHT" (sub (sget "SHT" "0") n))
                (sput stk (add (sget stk "0") bnew))
                (let ((p (near/promise_batch_create (near/current_account_id))))
                  (begin
                    (near/promise_batch_action_function_call p "wd-cb"
                      (json-set (json-set "{}" "w" who) "b" bnew)
                      "0" 60000000000000)
                    (near/promise_return p)
                    (near/log (str-cat "wd parked=" bnew))
                    (near/return "wd-queued")))))))))

;; ── wd-cb: success branch ONLY delivers stranded → paid ──
(define (wd-cb)
  (if (str= (near/predecessor_account_id) (near/current_account_id))
      (let* ((who (default (near/json_get_str "w") ""))
             (stk (str-cat "str:" who))
             (kp (str-cat "paid:" who))
             (parked (sget stk "0")))
        (begin
          (sput kp (add (sget kp "0") parked))
          (sput stk "0")
          (near/log (str-cat "wd-cb paid=" parked))
          (near/return "paid")))
      (near/return "forbidden")))

;; ── StrandPay: manual recovery of a parked credit ──
(define (strand-pay)
  (let* ((who (default (near/json_get_str "w") (near/predecessor_account_id)))
         (stk (str-cat "str:" who))
         (kp (str-cat "paid:" who))
         (parked (sget stk "0")))
    (begin
      (sput kp (add (sget kp "0") parked))
      (near/storage_remove stk)
      (near/log (str-cat "strand-pay recovered=" parked))
      (near/return parked))))

;; ── views: book(a) = bal + paid + pending + stranded (the theorem's LHS) ──
(define (book)
  (let* ((who (default (near/json_get_str "w") (near/predecessor_account_id)))
         (bal (sget (str-cat "bal:" who) "0"))
         (paid (sget (str-cat "paid:" who) "0"))
         (str (sget (str-cat "str:" who) "0"))
         (pend (settle who)))
    (near/return (add bal (add paid (add str pend))))))

(define (get-fees)
  (near/return (sget "fees" "0")))

(define (get-grows)
  (near/return (sget "grows" "0")))

(define (get-paid)
  (near/return (sget (str-cat "paid:" (near/predecessor_account_id)) "0")))

(define (get-stranded)
  (near/return (sget (str-cat "str:" (near/predecessor_account_id)) "0")))

(export "grow" grow)
(export "register" register)
(export "deposit" deposit)
(export "claim" claim)
(export "withdraw" withdraw)
(export "wd-cb" wd-cb)
(export "strand-pay" strand-pay)
(export "book" book)
(export "get-fees" get-fees)
(export "get-grows" get-grows)
(export "get-paid" get-paid)
(export "get-stranded" get-stranded)
