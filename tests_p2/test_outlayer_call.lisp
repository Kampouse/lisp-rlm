;; Test: outlayer/call builtin - signs and broadcasts a transaction
;; Requires: inlayer run --signer <account> --rpc https://rpc.mainnet.fastnear.com
;; Expected: tx hash string (e.g. "ARyC53ww...")
(define (run)
  (let* ((args "{\"account_id\":\"kampouse.near\"}")
         (result (outlayer/call "contract.main.burrow.near" "get_account" args "0" "300000000000000")))
    result))
