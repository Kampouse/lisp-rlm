;; token.lisp — minimal NEP-141-shaped fungible token for the mock.
;;
;; Purpose: give the fee-ledger a REAL payout leg. Instead of a self-call
;; receipt, the ledger's wd-cb fires an ft_transfer receipt at this
;; contract, so "paid" on the ledger means "token balance moved".
;;
;; NEP-141 surface kept to what the mock exercises: ft_transfer (escrow
;; model, predecessor = owner), ft_balance_of, ft_total_supply. Storage
;; model: bal:<acct> ASCII decimal u128, supply counter.
;;
;; Roles:
;;   init       (owner only, once): total supply to the owner's balance.
;;   ft_transfer: predecessor pays receiver_id amount. Owner pays from
;;              bal; escrow (the ledger) pays from escrow bal.
;;   ft_balance_of / ft_total_supply: JSON views.

(define (sget k d) (default (near/storage_get k) d))
(define (sput k v) (near/storage_set k v))
(define (fail m) (begin (near/log m) (near/abort m)))
(define (add a b) (u128/add a b))
(define (sub a b) (u128/sub a b))
(define (ult a b) (u128/lt a b))
(define (eq0 a) (u128/eq a "0"))

(define OWNER "fl-owner.test.near")
(define LEDGER "fl.clmm.test.near")

;; ── init: mint SUPPLY to OWNER (guard: double-init traps) ──
(define (init)
  (let ((sup (default (near/json_get_str "supply") "0")))
    (if (eq0 sup) (fail "ERR_SUPPLY")
        (if (not (eq0 (sget "supply" "0"))) (fail "ERR_INIT")
            (let ((b (str-cat "bal:" OWNER)))
              (begin
                (sput b sup)
                (sput "supply" sup)
                (near/log (str-cat "init supply=" sup))
                (near/return "ok")))))))

;; ── ft_transfer {receiver_id, amount} — escrow model ──
(define (ft_transfer)
  (let* ((from (near/predecessor_account_id))
         (to (default (near/json_get_str "receiver_id") ""))
         (amt (default (near/json_get_str "amount") "0"))
         (fb (str-cat "bal:" from))
         (tb (str-cat "bal:" to)))
    (if (eq0 amt) (fail "ERR_AMT")
        (if (str= to "") (fail "ERR_RCV")
            (if (ult (sget fb "0") amt) (fail "ERR_BAL")
                (begin
                  (sput fb (sub (sget fb "0") amt))
                  (sput tb (add (sget tb "0") amt))
                  (near/log (str-cat "xfer " from "->" to " " amt))
                  (near/return "ok")))))))

;; ── views ──
(define (ft_balance_of)
  (near/return (sget (str-cat "bal:" (default (near/json_get_str "account_id") "")) "0")))
(define (ft_total_supply)
  (near/return (sget "supply" "0")))

(export "init" init)
(export "ft_transfer" ft_transfer)
(export "ft_balance_of" ft_balance_of)
(export "ft_total_supply" ft_total_supply)
