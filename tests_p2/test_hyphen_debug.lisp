(define (run)
  (let* (
    (prices (http-get "https://api.rhea.finance/list-token-price"))
    ;; Extract both with hyphens
    (lst-obj (json-get-str "lst.rhealab.near" prices))
    (meta-obj (json-get-str "meta-pool.near" prices))
    (lst-p (json-get-str "price" lst-obj))
    (meta-p (json-get-str "price" meta-obj))
    )
    (str-cat "{\"lst_obj\":\"" lst-obj "\",\"meta_obj\":\"" meta-obj "\",\"lst_price\":\"" lst-p "\",\"meta_price\":\"" meta-p "\"}")))