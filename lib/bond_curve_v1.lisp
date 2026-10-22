;; bond_curve_v1 — constant-product launch curve with internal NEP-141-style
;; ledger. Reuses the stack's proven doors: quote arrives via ft_on_transfer
;; (token-verified predecessor), graduation seeds the CLMM pool through its
;; seed escrow, sell pays out through a promise with a reversal arm.
;;
;; Curve (pump-style virtual reserves):
;;   x = V + R          (QT side: virtual + real reserve)
;;   y = SUP − SOLD     (tokens still on the curve)
;;   buy  q → t = y·q/(x+q)          R += q, SOLD += t, b:buyer += t
;;   sell t → q = x·t/(y+t)          R −= q, SOLD −= t, b:seller −= t
;;   price = x/y; graduation when R ≥ G.
;;
;; Graduation: permissionless, two flat stages (measured-ladder laws: one
;; promise_result per then, literal attaches, slack ≥ 3T):
;;   stage 1 (graduate): pool.ft_on_transfer {sender: self, amount: y,
;;             msg seed} → ong1. Predecessor = curve = the pool's TOKX ⇒
;;             ESCX += y. Result empty ⇒ abort, curve untouched.
;;   stage 2 (ong1):     QT.ft_transfer_call → pool {a:"seed"} → ong2.
;;             ⇒ ESCY += R; the QT's resolve refunds if the pool rejects.
;;   ong2: both receipts were green along the chain ⇒ freeze forever:
;;             buys refund, sells/graduate reject. The y tokens were never
;;             in anyone's ledger; the pool float enters circulation via
;;             swaps (ft_transfer special-cases the pool as sender). No
;;             mint verb exists — the circulating supply is fixed at the
;;             frozen SOLD, and the pool LP is burn-locked by the pool.
;;
;; All money math is u128-muldiv (wide-limb) in the string domain.
;; State (prefix C|): QT, POOL, V, G, R, SUP, SOLD, GRAD
;; Ledger: b:<acct>. ft surface: ft_balance_of, ft_total_supply, ft_transfer.

(define (c-bz k) (let ((v (near/load-bytes k))) (if (nil? v) "0" v)))
(define (c-put k v) (near/store-bytes k v))
(define (c-fail m) (near/abort m))
(define (c-bal who) (c-bz (str-cat "b:" who)))
(define (c-x) (li-add (c-bz "C|V") (c-bz "C|R")))
(define (c-y) (li-sub (c-bz "C|SUP") (c-bz "C|SOLD")))

;; ── init {qt, pool, sup, v, g} — once-ever ───────────────────────────────
(define (c-init)
  (if (not (nil? (near/load-bytes "C|QT")))
      "already-initialized"
      (let ((inp (near/input)))
        (begin
          (c-put "C|QT" (json-get-str "qt" inp))
          (c-put "C|POOL" (json-get-str "pool" inp))
          (c-put "C|SUP" (json-get-str "sup" inp))
          (c-put "C|V" (json-get-str "v" inp))
          (c-put "C|G" (json-get-str "g" inp))
          (c-put "C|LO" (json-get-str "lo" inp))
          (c-put "C|HI" (json-get-str "hi" inp))
          (c-put "C|LOCK" (json-get-str "lock" inp))
          (c-put "C|R" "0")
          (c-put "C|SOLD" "0")
          (c-put "C|GRAD" "")
          "init-ok"))))

;; ── quote views (pure: same formulas as the legs) ────────────────────────
(define (c-quote-buy)
  (let ((inp (near/input)))
    (near/return (u128-muldiv (c-y)
                              (json-get-str "q" inp)
                              (li-add (c-x) (json-get-str "q" inp))))))
(define (c-quote-sell)
  (let ((inp (near/input)))
    (near/return (u128-muldiv (c-x)
                              (json-get-str "t" inp)
                              (li-add (c-y) (json-get-str "t" inp))))))

;; ── packed state view ────────────────────────────────────────────────────
(define (c-state)
  (str-cat "qt:" (str-cat (c-bz "C|QT")
  (str-cat ":pool:" (str-cat (c-bz "C|POOL")
  (str-cat ":sup:" (str-cat (c-bz "C|SUP")
  (str-cat ":x:" (str-cat (c-x)
  (str-cat ":y:" (str-cat (c-y)
  (str-cat ":r:" (str-cat (c-bz "C|R")
  (str-cat ":sold:" (str-cat (c-bz "C|SOLD")
  (str-cat ":grad:" (c-bz "C|GRAD")))))))))))))))))

;; ── buy door: predecessor MUST be the quote token, msg a:"buy" ───────────
(define (c-on-transfer)
  (let ((inp (near/input)))
    (let ((sender (json-get-str "sender_id" inp)))
      (let ((amt (json-get-str "amount" inp)))
        (if (not (str= (near/predecessor_account_id) (c-bz "C|QT")))
            (near/return amt)
            (if (not (str= (c-bz "C|GRAD") ""))
                (near/return amt)
                (if (not (str= (json-get-str "a" (json-get-str "msg" inp)) "buy"))
                    (near/return amt)
                    ;; t = y·q/(x+q); reject dust and oversell
                    (let ((t (u128-muldiv (c-y) amt (li-add (c-x) amt))))
                      (if (or (not (= (li-cmp t "0") 1))
                              (not (= (li-cmp (c-y) t) 1)))
                          (near/return amt)
                          (begin
                            (c-put (str-cat "b:" sender)
                                   (li-add (c-bal sender) t))
                            (c-put "C|SOLD" (li-add (c-bz "C|SOLD") t))
                            (c-put "C|R" (li-add (c-bz "C|R") amt))
                            (near/return "0")))))))))))

;; ── sell {t} — debit, pay via promise, onsell re-credits on failure ──────
(define (c-sell)
  (let ((inp (near/input)))
  (let ((who (near/predecessor_account_id)))
  (let ((t (json-get-str "t" inp)))
    (if (not (str= (c-bz "C|GRAD") ""))
        (c-fail "ERR_GRAD")
        (if (not (= (li-cmp (c-bal who) t) 1))
            (c-fail "ERR_BAL")
            (let ((q (u128-muldiv (c-x) t (li-add (c-y) t))))
              (if (not (= (li-cmp q "0") 1))
                  (c-fail "ERR_DUST")
                  (begin
                    (c-put (str-cat "b:" who) (li-sub (c-bal who) t))
                    (c-put "C|SOLD" (li-sub (c-bz "C|SOLD") t))
                    (c-put "C|R" (li-sub (c-bz "C|R") q))
                    (let ((p (near/promise_batch_create (c-bz "C|QT"))))
                      (begin
                        (near/promise_batch_action_function_call p
                          "ft_transfer"
                          (json-set (json-set "{}" "receiver_id" who)
                                    "amount" q)
                          "0" 10000000000000)
                        (near/promise_return
                          (near/promise_then p (near/current_account_id)
                            "onsell"
                            (json-set (json-set (json-set "{}"
                                        "who" who) "t" t) "q" q)
                            "0" 5000000000000))
                        "sell:")))))))))))

;; onsell — empty result ⇒ the QT transfer died: restore books exactly.
(define (c-onsell)
  (if (< (str-length (near/promise_result 0)) 1)
      (let ((inp (near/input)))
        (let ((who (json-get-str "who" inp)))
          (let ((t (json-get-str "t" inp)))
            (let ((q (json-get-str "q" inp)))
              (begin
                (c-put (str-cat "b:" who) (li-add (c-bal who) t))
                (c-put "C|SOLD" (li-add (c-bz "C|SOLD") t))
                (c-put "C|R" (li-add (c-bz "C|R") q))
                "sell-refunded")))))
      "sell-ok"))

;; ── payable buy — attach q yoctoNEAR directly, no wnear registration ─────
;; Guards run BEFORE the wrap: graduated / dust / oversell abort
;; synchronously and the attached deposit refunds automatically (nothing
;; was scheduled). The wrap (QT.near_deposit) attaches q itself — its own
;; failure refunds its attachment, and onbuy aborts on an empty result.
;; onbuy re-runs the credit guards: promise callbacks serialize AFTER any
;; racing buys/graduate, so a book change between buy and onbuy is caught
;; there — the wrapped wnear then goes back via ft_transfer, with an
;; rb:<who> ledger + claim_refund if that transfer dies (unregistered
;; receiver).
(define (c-buy)
  (let ((who (near/predecessor_account_id)))
  (let ((q (near/attached_deposit_u128)))
    (if (not (str= (c-bz "C|GRAD") ""))
        (c-fail "ERR_GRAD")
        (let ((t (u128-muldiv (c-y) q (li-add (c-x) q))))
          (if (or (not (= (li-cmp t "0") 1))
                  (not (= (li-cmp (c-y) t) 1)))
              (c-fail "ERR_DUST")
              (let ((p (near/promise_batch_create (c-bz "C|QT"))))
                (begin
                  (near/promise_batch_action_function_call p
                    "near_deposit" "{}" q 10000000000000)
                  (near/promise_return
                    (near/promise_then p (near/current_account_id)
                      "onbuy"
                      (json-set (json-set "{}" "who" who) "q" q)
                      "0" 40000000000000))
                  "buy:"))))))))

;; onbuy — wrap confirmed ⇒ credit. Race re-checks (see c-buy); the
;; credit order matches c-on-transfer exactly (b:, SOLD, R).
(define (c-onbuy)
  (let ((inp (near/input)))
  (let ((who (json-get-str "who" inp)))
  (let ((q (json-get-str "q" inp)))
    (if (< (str-length (near/promise_result 0)) 1)
        (c-fail "ERR_WRAP")
        (let ((t (u128-muldiv (c-y) q (li-add (c-x) q))))
          (if (or (not (str= (c-bz "C|GRAD") ""))
                  (not (= (li-cmp t "0") 1))
                  (not (= (li-cmp (c-y) t) 1)))
              (let ((p (near/promise_batch_create (c-bz "C|QT"))))
                (begin
                  (near/promise_batch_action_function_call p
                    "ft_transfer"
                    (json-set (json-set "{}" "receiver_id" who)
                              "amount" q)
                    "1" 10000000000000)
                  (near/promise_return
                    (near/promise_then p (near/current_account_id)
                      "onref"
                      (json-set (json-set "{}" "who" who) "q" q)
                      "0" 5000000000000))
                  "buy-refunding:"))
              (begin
                (c-put (str-cat "b:" who) (li-add (c-bal who) t))
                (c-put "C|SOLD" (li-add (c-bz "C|SOLD") t))
                (c-put "C|R" (li-add (c-bz "C|R") q))
                "bought:"))))))))

;; onref — refund died (receiver unregistered with the QT): park it in
;; the rb ledger for claim_refund. The wnear stays in the curve's ledger.
(define (c-onref)
  (if (< (str-length (near/promise_result 0)) 1)
      (let ((inp (near/input)))
        (let ((who (json-get-str "who" inp)))
          (let ((q (json-get-str "q" inp)))
            (begin
              (c-put (str-cat "rb:" who) (li-add (c-bz (str-cat "rb:" who)) q))
              "refund-parked:"))))
      "refund-ok"))

;; claim_refund {} — pay a parked refund back (any time, live or frozen).
(define (c-claim-refund)
  (let ((who (near/predecessor_account_id)))
    (let ((q (c-bz (str-cat "rb:" who))))
      (if (not (= (li-cmp q "0") 1))
          "no-refund"
          (let ((p (near/promise_batch_create (c-bz "C|QT"))))
            (begin
              (c-put (str-cat "rb:" who) "0")
              (near/promise_batch_action_function_call p
                "ft_transfer"
                (json-set (json-set "{}" "receiver_id" who) "amount" q)
                "1" 10000000000000)
              (near/promise_return
                (near/promise_then p (near/current_account_id)
                  "onref2"
                  (json-set (json-set "{}" "who" who) "q" q)
                  "0" 5000000000000))
              "refund:"))))))

;; onref2 — claim_refund's transfer died: re-park (retryable forever).
(define (c-onref2)
  (if (< (str-length (near/promise_result 0)) 1)
      (let ((inp (near/input)))
        (let ((who (json-get-str "who" inp)))
          (let ((q (json-get-str "q" inp)))
            (begin
              (c-put (str-cat "rb:" who) (li-add (c-bz (str-cat "rb:" who)) q))
              "refund-parked:"))))
      "refund-ok"))

;; ── graduate — permissionless, requires R ≥ G, live curve ────────────────
;; Stage 1: seed the y float into the pool (direct fn_call on the pool's
;; ft_on_transfer — predecessor = curve = the pool's TOKX). Then ong1.
(define (c-graduate)
  (if (not (str= (c-bz "C|GRAD") ""))
      (c-fail "ERR_STATE")
      (if (not (= (li-cmp (c-bz "C|R") (c-bz "C|G")) 1))
          (c-fail "ERR_NOT_READY")
          (let ((p (near/promise_batch_create (c-bz "C|POOL"))))
            (begin
              (near/promise_batch_action_function_call p
                "ft_on_transfer"
                (json-set (json-set (json-set "{}"
                            "sender_id" (near/current_account_id))
                            "amount" (c-y))
                            "msg" "{\"a\":\"seed\"}")
                "0" 30000000000000)
              (near/promise_return
                (near/promise_then p (near/current_account_id)
                  "ong1"
                  "{}" "0" 160000000000000))
              "graduating:")))))

;; ong1 — stage 2: seed the real QT reserve via the quote token's own
;; transfer_call (its resolve refunds into R's source if the pool rejects).
(define (c-ong1)
  (if (< (str-length (near/promise_result 0)) 1)
      (c-fail "ERR_SEEDX")
      (let ((q (near/promise_batch_create (c-bz "C|QT"))))
        (begin
          (near/promise_batch_action_function_call q
            "ft_transfer_call"
            (json-set (json-set (json-set "{}"
                        "receiver_id" (c-bz "C|POOL"))
                        "amount" (c-bz "C|R"))
                        "msg" "{\"a\":\"seed\"}")
            "0" 80000000000000)
          (near/promise_return
            (near/promise_then q (near/current_account_id)
              "ong2" "{}" "0" 70000000000000))
          "seedy:"))))

;; ong2 — stage 3: mint the graduated LP position from the seeded escrow
;; custody (the pool solves d from its own escrow books; lock_secs arms
;; its burn-until in the same receipt).
(define (c-ong2)
  (if (< (str-length (near/promise_result 0)) 1)
      (c-fail "ERR_SEEDY")
      (let ((p (near/promise_batch_create (c-bz "C|POOL"))))
        (begin
          (near/promise_batch_action_function_call p
            "provision"
            (json-set (json-set (json-set "{}"
                        "lo" (c-bz "C|LO"))
                        "hi" (c-bz "C|HI"))
                      "lock_secs" (c-bz "C|LOCK"))
            "0" 60000000000000)
          (near/promise_return
            (near/promise_then p (near/current_account_id)
              "ong3" "{}" "0" 5000000000000))
          "provisioning:"))))

;; ong3 — all three legs green ⇒ freeze forever. The curve never touches
;; its books during graduation (every leg lands on the POOL side), so an
;; abort leaves the curve exactly as it was.
(define (c-ong3)
  (if (< (str-length (near/promise_result 0)) 1)
      (c-fail "ERR_PROV")
      (begin
        (c-put "C|GRAD" "1")
        "grad-ok")))

;; ── NEP-141 surface (ledger only; the pool pays out of the seeded float) ──
(define (c-balance)
  (near/return (c-bal (json-get-str "account_id" (near/input)))))
(define (c-total-supply)
  (near/return (c-bz "C|SOLD")))

;; plain transfer. sender = predecessor. The POOL has no ledger entry — its
;; balance IS the seeded float (post-grad out-legs), so pool-initiated
;; transfers credit the receiver without a debit.
(define (c-transfer)
  (let ((inp (near/input)))
    (let ((who (near/predecessor_account_id)))
      (let ((to (json-get-str "receiver_id" inp)))
        (let ((amt (json-get-str "amount" inp)))
          (if (not (= (li-cmp amt "0") 1))
              (near/return "bad-amt")
              (if (str= who (c-bz "C|POOL"))
                  (begin
                    (c-put (str-cat "b:" to) (li-add (c-bal to) amt))
                    (near/return "ok"))
                  (if (not (= (li-cmp (c-bal who) amt) 1))
                      (near/return "bad-bal")
                      (begin
                        (c-put (str-cat "b:" who) (li-sub (c-bal who) amt))
                        (c-put (str-cat "b:" to) (li-add (c-bal to) amt))
                        (near/return "ok"))))))))))

(export "init" c-init)
(export "state" c-state)
(export "quote_buy" c-quote-buy)
(export "quote_sell" c-quote-sell)
(export "ft_on_transfer" c-on-transfer)
(export "sell" c-sell)
(export "buy" c-buy)
(export "onbuy" c-onbuy)
(export "onref" c-onref)
(export "claim_refund" c-claim-refund)
(export "onref2" c-onref2)
(export "onsell" c-onsell)
(export "graduate" c-graduate)
(export "ong1" c-ong1)
(export "ong2" c-ong2)
(export "ong3" c-ong3)
(export "ft_balance_of" c-balance)
(export "ft_total_supply" c-total-supply)
(export "ft_transfer" c-transfer)
