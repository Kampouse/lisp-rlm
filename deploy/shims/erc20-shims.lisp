
;; ── NEAR method shims (deploy surface — appended by gen_deploy.py) ──
;; Mutating ops take the acting account from the TRANSACTION
;; (predecessor = ERC-20 msg.sender), never from args. Amounts are
;; u128 decimal strings in the input JSON. Success: non-empty string
;; return; failure: "" (corpus error model).
(define (m-ft-mint)
  (near/json_return_str
    (ft-mint (near/json_get_str "to") (near/json_get_str "amount"))))

(define (m-ft-transfer)
  (near/json_return_str
    (ft-transfer (near/predecessor_account_id)
                 (near/json_get_str "to") (near/json_get_str "amount"))))

(define (m-ft-approve)
  (near/json_return_str
    (ft-approve (near/predecessor_account_id)
                (near/json_get_str "spender") (near/json_get_str "amount"))))

(define (m-ft-transfer-from)
  (near/json_return_str
    (ft-transfer-from (near/predecessor_account_id)
                      (near/json_get_str "from") (near/json_get_str "to")
                      (near/json_get_str "amount"))))

(define (m-ft-balance-of)
  (near/json_return_str (balance-of (near/json_get_str "account"))))

(define (m-ft-total-supply)
  (near/json_return_str (total-supply)))

(export "ft_mint" m-ft-mint false)
(export "ft_transfer" m-ft-transfer false)
(export "ft_approve" m-ft-approve false)
(export "ft_transfer_from" m-ft-transfer-from false)
(export "ft_balance_of" m-ft-balance-of true)
(export "ft_total_supply" m-ft-total-supply true)
