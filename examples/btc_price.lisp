;; btc_price.lisp — live outlayer demo: fetch BTC/USD and extract the price.
;; Run: ./target/debug/outlayer-run examples/btc_price.lisp
(define (fetch-spot)
  (http-get "https://api.coinbase.com/v2/prices/BTC-USD/spot"))

(define (run)
  (let ((resp (fetch-spot)))
    (json-get "data.amount" resp)))
