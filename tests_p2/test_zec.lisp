(define (run)
  (let* ((prices (http-get "https://api.rhea.finance/list-token-price"))
         (zec-obj (json-get-str "zec.omft.near" prices)))
    (str-cat "ZEC obj len: " (to-string (str-len zec-obj)) " val: " zec-obj)))
