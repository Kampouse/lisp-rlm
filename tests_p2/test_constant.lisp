(define (run)
  (let* (
    (_ (http-get "https://api.rhea.finance/list-token-price"))
    (args "{\"account_id\":\"nonexistent.near\"}")
    (account-raw (outlayer/view "contract.main.burrow.near" "get_account" args))
    ;; Just return constant - ignore account-raw entirely
    )
    "{\"status\":\"ok\",\"account\":\"nonexistent\"}"))