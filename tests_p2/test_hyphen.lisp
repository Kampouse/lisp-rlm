(define (run)
  (let* (
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    ;; Keys with hyphens
    (meta-raw (json-get-str "meta-pool.near" prices))
    ;; Keys without hyphens
    (lst-raw (json-get-str "lst.rhealab.near" prices))
    )
    (str-cat "{\"meta_pool\":\"" meta-raw "\",\"lst_pool\":\"" lst-raw "\"}")))