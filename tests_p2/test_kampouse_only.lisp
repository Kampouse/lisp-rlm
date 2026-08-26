(define (run)
  (let* (
    (_ (http-get "https://api.rhea.finance/list-token-price"))
    ;; Try kampouse.near which EXISTS
    (args "{\"account_id\":\"kampouse.near\"}")
    (account-raw (outlayer/view "contract.main.burrow.near" "get_account" args))
    )
    "{\"status\":\"ok\"}"))