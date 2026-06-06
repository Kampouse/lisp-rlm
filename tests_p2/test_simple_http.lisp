;; Test with substr and str-len to check http-get works
(define (run)
  (let* ((near-json (http-get "https://api.rhea.finance/get-token-price?token_id=wrap.near")))
    (str-cat "Len: " (str-len near-json) " | First: " (substr near-json 0 50))))