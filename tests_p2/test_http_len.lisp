;; Test HTTP GET return value
(define (run)
  (let* ((near-json (http-get "https://api.rhea.finance/get-token-price?token_id=wrap.near"))
         (len (str-len near-json)))
    (str-cat "Response length: " len "\n"
             "First 100: " (substr near-json 0 100))))