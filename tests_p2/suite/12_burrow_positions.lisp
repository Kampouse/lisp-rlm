(define (run)
  (let* (
    (pos (outlayer/view "contract.main.burrow.near" "get_account" "{\"account_id\":\"kampouse.near\"}"))
    (acct (json-get-str "account_id" pos))
    )
    acct))
