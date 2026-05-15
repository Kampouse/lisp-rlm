;; Test 8: json-get with nested fields and multiple queries
(define (main)
  (print (json-get "name"))
  (print (json-get "price"))
  (print (+ (json-get "price") 100)))
;; Input: {"name": "BTC", "price": 42000}
;; Expected: BTC 42000 42100
