;;;
;;; price-consumer — near/call-await demo against the live btc-oracle.
;;;
;;; The ENTIRE cross-contract call + callback wiring is one form:
;;;   (near/call-await TARGET "get_fresh" ARGS GAS "on_price" CB_GAS CB_ARGS)
;;; CB_ARGS is passed to the callback as its INPUT json — that's how you
;;; correlate multiple outstanding calls (here: a request tag).
;;; The callback reads the result via (near/promise_result 0) — fail-closed:
;;; "" if the callee errored (checked via str-length, mirrors oracle's own
;;; stale guard).
;;;
;;; Methods:
;;;   pull      — fire the call, register on_price with tag "pull-1"
;;;   pull_bad  — same but targeting a method that doesn't exist (failure demo)
;;;   on_price  — result lands here: stores "tag|json" at "last"
;;;   last      — view the stored result

(define TARGET "btc-oracle.kampy.test.near")
(define TTL "{\"ttl\":\"300\"}")
(define CBGAS 20000000000000)
(define GAS 20000000000000)

(define (pull)
  (near/call-await TARGET "get_fresh" TTL GAS "on_price" CBGAS "{\"tag\":\"pull-1\"}"))

(define (pull_bad)
  (near/call-await TARGET "no_such_method" TTL GAS "on_price" CBGAS "{\"tag\":\"pull-bad\"}"))

(define (on_price)
  (begin
    (let ((tag (near/json_get_str "tag"))
          ;; storage_get yields (opt str) — (default ...) unwraps, "" on miss;
          ;; promise_result is fail-closed: "" when the callee errored.
          (res (default (near/promise_result 0) "")))
      (if (= (str-length res) 0)
          (near/storage_set "last" (str-cat tag "|FAIL"))
          (near/storage_set "last" (str-cat tag (str-cat "|" res)))))
    0))

(define (last)
  (near/return_str (default (near/storage_get "last") "")))

(export "pull" pull #f)
(export "pull_bad" pull_bad #f)
(export "on_price" on_price #f)
(export "last" last #t)
