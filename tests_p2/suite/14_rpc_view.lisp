(define (run)
  (let* (
    (pos (outlayer/view "contract.main.burrow.near" "get_account" "{\"account_id\":\"kampouse.near\"}"))
    (supplied (json-get-str "supplied" pos))
    (price (http-get "https://api.rhea.finance/list-token-price"))
    (usdt (json-get-str "price" (json-get-str "usdt.tether-token.near" price)))
    (out (str-cat "{\"usdt_supply\":\"" supplied "\",\"usdt_price\":\"" usdt "\"}"))
    )
    out))
