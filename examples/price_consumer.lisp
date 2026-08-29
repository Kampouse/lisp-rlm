;;;
;;; price-consumer — near/call-await demo against the live btc-oracle.
;;;
;;; The ENTIRE cross-contract call + callback wiring is one form:
;;;   (near/call-await TARGET "get_fresh" ARGS GAS "on_price" CB_GAS)
;;; Compare: v1-style consumers needed 5 builtin calls + manual
;;; promise_then idx juggling (see outlayer-oracle refresh).
;;;
;;; Methods:
;;;   pull     — fire the call, register on_price as callback
;;;   on_price — result lands here: stores raw JSON at "last"
;;;   last     — view the stored JSON

(define TARGET "btc-oracle.kampy.testnet")
(define TTL "{\"ttl\":\"300\"}")

(define (pull)
  (near/call-await TARGET "get_fresh" TTL 20000000000000 "on_price" 20000000000000))

(define (on_price)
  (begin
    (near/storage_set "last" (near/promise_result 0))
    0))

(define (last)
  (near/return_str (near/storage_get "last")))

(export "pull" pull #f)
(export "on_price" on_price #f)
(export "last" last #t)
