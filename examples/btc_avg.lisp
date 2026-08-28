;; btc_avg.lisp — multi-source BTC/USD average via the OutLayer.
;; Sources: Hyperliquid (allMids flat map, POST), Coinbase (spot, GET), Bitstamp (ticker, GET).
;; Run: ./target/debug/outlayer-run examples/btc_avg.lisp
;;
;; Prices arrive as decimal strings ("117468.5"). str->num skips the '.',
;; so dollars = str->num of the part BEFORE the first dot:
;;   (nth (str-split s ".") 0)

(define (dollars s)
  (str->num (nth (str-split s ".") 0)))

(define (hyperliquid-btc)
  (dollars (json-get "BTC"
    (http-post "https://api.hyperliquid.xyz/info" "{\"type\":\"allMids\"}"))))

(define (coinbase-btc)
  (dollars (json-get "data.amount"
    (http-get "https://api.coinbase.com/v2/prices/BTC-USD/spot"))))

(define (bitstamp-btc)
  (dollars (json-get "last"
    (http-get "https://www.bitstamp.net/api/v2/ticker/btcusd/"))))

(define (run)
  (let ((hl (hyperliquid-btc))
        (cb (coinbase-btc))
        (bs (bitstamp-btc)))
    (begin
      (println (str-cat "hyperliquid: " (to-string hl)))
      (println (str-cat "coinbase:    " (to-string cb)))
      (println (str-cat "bitstamp:    " (to-string bs)))
      (/ (+ (+ hl cb) bs) 3))))
