;;;
;;; outlayer-oracle v2 — P1 NEAR contract that consumes an OutLayer P2 execution.
;;;
;;; A lisp contract calls OutLayer to run a lisp-compiled wasi:http component
;;; (btc_avg_json.wasm) inside a TEE, and stores the stdout JSON back into
;;; NEAR storage — one language, end to end.
;;;
;;; v2 additions over v1:
;;;   "ts"     — block timestamp (ns) of the callback that stored the result
;;;   "count"  — run counter as decimal STRING (store_num tagged-word didn't
;;;              round-trip across txs; the string-safe family does)
;;;   "h:<i>"  — history ring, 50 slots (i in 0..49), wraps forever
;;;
;;; Methods:
;;;   refresh           — request OutLayer execution, callback stores result
;;;   get_oracle        — view the stored JSON (raw, always latest)
;;;   get_ts            — view the timestamp (ns) of the last store
;;;   get_runs          — view the run counter
;;;   get_fresh {"ttl":"600"} — JSON if age <= ttl seconds, else {"error":"stale"}
;;;   get_history {"n":"5"}   — last n results, "|" separated, newest first
;;;
;;; LANDMINE COMPLIANCE (GAPS.md): no lambdas/closures (T4); explicit
;;; numeric compares only (0 is truthy); nested BINARY str-cat for composite
;;; keys; all state via near/storage_set/get (string-safe); guarded str->num
;;; (never feeds "" into arithmetic); 49 hardcoded (literal defines are not
;;; inlined on all paths).
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

;; ── storage helpers ───────────────────────────────────────────────
(define (get-str k) (near/storage_get k))

(define (get-count)
  (let ((v (get-str "count")))
    (if (= (str-length v) 0) "0" v)))

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

;; ── on_result: OutLayer's stdout lands here ───────────────────────
;; Stores: latest JSON ("oracle"), its ts ("ts"), history ring slot
;; ("h:<i>"), and the bumped counter ("count", wraps 0..49).
;; ts stored DIRECTLY: near/block_timestamp already returns an exact
;; decimal-string of ns (2026-08-26 Option A ruling — ns > 61-bit tagged
;; payload). NEVER to-string/str->num it — to-string untags as num and
;; stores heap-POINTER garbage.
(define (on_result)
  (let ((out (near/promise_result 0))
        (c (str->num (get-count))))
    (near/storage_set "oracle" out)
    (near/storage_set "ts" (near/block_timestamp))
    (near/storage_set (str-cat "h:" (to-string c)) out)
    (near/storage_set "count" (to-string (if (= c 49) 0 (+ c 1))))
    (near/log (str-cat "outlayer-oracle v2: " out))
    0))

;; ── views ─────────────────────────────────────────────────────────
(define (get_oracle)
  (near/return_str (get-str "oracle")))

(define (get_ts)
  (near/json_return_str (get-str "ts")))

(define (get_runs)
  (near/json_return_str (get-count)))

;; get_fresh {"ttl": "600"} — newest JSON if its age <= ttl seconds,
;; else {"error":"stale"}. All ns math in u128 STRINGS (Option A idiom):
;; block_timestamp IS a decimal string; u128/sub/gt/mul are exact.
;; LANDMINE: nested u128/* calls as ARGUMENTS collide on the shared
;; TEMP_MEM scratch (u128/gt X (u128/mul Y Z) miscompares) — intermediate
;; u128 results are ALWAYS let-bound (verified via near-mock probes).
;; Guards: missing oracle OR missing ts → stale (u128/sub "" would
;; hard-error; nested ifs keep it unreachable).
(define (get_fresh)
  (let ((ttl (near/json_get_str "ttl"))
        (out (get-str "oracle")))
    (if (= (str-length out) 0)
        (near/json_return_str "{\"error\":\"stale\"}")
        (let ((ts (get-str "ts")))
          (if (= (str-length ts) 0)
              (near/json_return_str "{\"error\":\"stale\"}")
              (let ((limit (u128/mul ttl "1000000000"))
                    (age (u128/sub (near/block_timestamp) ts)))
                (if (u128/gt age limit)
                    (near/json_return_str "{\"error\":\"stale\"}")
                    (near/json_return_str out))))))))

;; get_history {"n": "5"} — newest first, "|" separated. Ring wrap: after
;; 50 runs, slots contain the newest overwrite plus older-cycle entries.
(define (hist-walk i rem acc)
  (if (or (= rem 0) (< i 0))
      acc
      (hist-walk (- i 1) (- rem 1)
                 (str-cat acc (str-cat "|" (get-str (str-cat "h:" (to-string i))))))))

(define (get_history)
  (let ((n (str->num (near/json_get_str "n")))
        (c (- (str->num (get-count)) 1)))
    (near/json_return_str (hist-walk c n ""))))

(export "refresh" refresh #f)
(export "on_result" on_result #f)
(export "get_oracle" get_oracle #t)
(export "get_ts" get_ts #t)
(export "get_runs" get_runs #t)
(export "get_fresh" get_fresh #t)
(export "get_history" get_history #t)
