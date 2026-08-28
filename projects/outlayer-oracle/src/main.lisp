;;;
;;; outlayer-oracle — P1 NEAR contract that consumes an OutLayer P2 execution.
;;;
;;; The world's most circular oracle: a lisp contract calls OutLayer to run a
;;; lisp-compiled wasi:http component (btc_avg_json.wasm) inside a TEE, and
;;; stores the stdout JSON back into NEAR storage — one language, end to end.
;;;
;;; Methods:
;;;   refresh    — request OutLayer execution, callback stores the result
;;;   get_oracle — view the stored JSON
;;;   get_runs   — view the callback counter
;;;
;;; Storage:
;;;   "oracle" : raw stdout JSON from the TEE run (store-bytes)
;;;   num key 1 : callback counter (store_num)
;;;

;; request_execution args for the btc_avg_json component (hash-pinned WasmUrl)
(define ARGS
  "{\"source\":{\"WasmUrl\":{\"url\":\"https://raw.githubusercontent.com/Kampouse/lisp-rlm/main/examples/btc_avg_json.wasm\",\"hash\":\"0eeb1b8747d5080e1d69f26603c61576ddb51793323ca8f9b518eb28ebae116d\",\"build_target\":\"wasm32-wasip2\"}},\"resource_limits\":{\"max_instructions\":10000000000,\"max_memory_mb\":128,\"max_execution_seconds\":30},\"input_data\":\"{}\",\"response_format\":\"Text\"}")

;; Deposit as a DECIMAL string of yoctoNEAR — the emitter runs it through
;; __u128_parse → 16-byte LE at TEMP_MEM (u128-capable, no byte-string games).
(define DEPOSIT "10000000000000000000000") ; 0.01 NEAR
(define ZERO    "0")
(define EMPTY "")

(define OUTLAYER "outlayer.testnet")

;; ── refresh: promise chain → OutLayer → back to SELF.on_result ──
(define (refresh)
  (let ((self (near/current_account_id)))
    (let ((idx (near/promise_batch_create OUTLAYER)))
      ;; outlayer.testnet.request_execution(ARGS, deposit=0.01N, gas=100T)
      (near/promise_batch_action_function_call
        idx "request_execution" ARGS DEPOSIT 100000000000000)
      ;; promise_batch_then RETURNS a new callback-promise idx — attach
      ;; on_result to THAT, not to the outlayer batch
      (let ((cb (near/promise_batch_then idx self)))
        (near/promise_batch_action_function_call
          cb "on_result" EMPTY ZERO 40000000000000)
        (near/promise_return cb)))))

;; ── on_result: OutLayer's stdout lands here ──
(define (on_result)
  (let ((out (near/promise_result 0)))
    (near/store-bytes "oracle" out)
    (near/store_num 1 (+ 1 (near/load_num 1)))
    (near/log (str-cat "outlayer-oracle: " out))
    0))

;; ── views ──
(define (get_oracle)
  (near/return_str (near/load-bytes "oracle")))

(define (get_runs)
  (near/load_num 1))

(export "refresh" refresh #f)
(export "on_result" on_result #f)
(export "get_oracle" get_oracle #t)
(export "get_runs" get_runs #t)
