;;;
;;; crosspost v2 — one call → FastNEAR KV entry + Nostr event signed under
;;; the CALLER's derived key (OutLayer project run of nostr-chat-agent).
;;;
;;;   alice → post(text) ─┬─ social.near.__fastdata_kv  {"post/<ms>": {…}}   (indexed)
;;;                       └─ outlayer.request_execution  {"op":"chat",…,caller} → TEE
;;;          ──▶ post_result (DEFERRED: fires when the worker RESOLVES the
;;;              execution — stdout = {"id","sig","pk","posted"}). v2 stores
;;;              the rebuilt NIP-01 event in an outbox for the WS relay
;;;              bridge (Cloudflare Worker) to publish.
;;;
;;; v2 changes (proven paths untouched):
;;;   - oracle-pattern callback (promise_batch_then, like outlayer-oracle v2)
;;;   - post_result reads near/promise_result 0 (execution stdout), parses
;;;     id/sig/pk via 2-arg json-get, rebuilds the FULL event JSON, stores:
;;;       "ev:<n>"    full NIP-01 event  (relay-ready)
;;;       "evn:<n>"   caller (identity trail)
;;;       "pk:<caller>" derived pubkey     (per-caller identity proof)
;;;       "ev-count"  decimal-string counter
;;;   - views: get_outbox {"from":"<n>"} → JSON array of unpublished events
;;;            get_count, get_pk {"caller"}, last, last-post
;;;   - fail-closed text guard: rejects `"` and `\` (v1 TODO) — kills the
;;;     JSON-injection hole in BOTH legs.
;;;
;;; Fee: caller attaches ≥ 0.012N; contract forwards 0.01N to OutLayer.
;;;

(define KV-TARGET      "social.near")                  ; __fastdata_kv namespace (mainnet indexer; executes only on mainnet)
(define OUTLAYER       "outlayer.testnet")             ; testnet OutLayer contract
(define OL-PROJECT     "xcross-9f3.testnet/nostr-chat") ; deployed project (TEE storage+VRF hosts)
(define GAS-OL         150000000000000)
(define GAS-SOCIAL     5000000000000)
(define GAS-CB         40000000000000)

(define (post)
  (let ((caller (near/predecessor_account_id))
        (ts     (near/block_timestamp))
        (text   (near/json_get_str "text")))
    ;; fail-closed: no quotes/backslash in text (JSON-injection guard)
    (if (< 0 (str-index-of text "\""))
        (near/panic "text contains quote")
    (if (< 0 (str-index-of text "\\"))
        (near/panic "text contains backslash")
    (let ((ts-s   (str-slice ts 0 10))
          (nonce  (str-slice ts 13 19))
          (room   "crosspost"))
    (let ((ol (near/promise_batch_create OUTLAYER)))
      (near/promise_batch_action_function_call ol "request_execution"
        (str-cat "{\"source\":{\"Project\":{\"project_id\":\"" OL-PROJECT
                 "\",\"version_key\":null}},"
                 "\"resource_limits\":{\"max_instructions\":5000000000,"
                 "\"max_memory_mb\":256,\"max_execution_seconds\":60},"
                 "\"input_data\":\"{\\\"op\\\":\\\"chat\\\",\\\"content\\\":\\\"" text
                 "\\\",\\\"caller\\\":\\\"" caller
                 "\\\",\\\"ts\\\":\\\"" ts-s
                 "\\\",\\\"nonce\\\":\\\"" nonce
                 "\\\",\\\"room\\\":\\\"" room
                 "\\\",\\\"r\\\":\\\"0\\\"}\","
                 "\"payer_account_id\":\"" caller "\","
                 "\"params\":{\"compile_only\":false}}")
        "10000000000000000000000" GAS-OL)
      ;; Leg 1 — FastNEAR KV (unchanged, fire-and-forget)
      (let ((b (near/promise_batch_create KV-TARGET)))
        (near/promise_batch_action_function_call b "__fastdata_kv"
          (str-cat "{\"post/" ts "\":{\"text\":\"" text "\",\"author\":\"" caller "\"}}")
          "0" GAS-SOCIAL))
      ;; oracle-pattern DEFERRED callback — fires at worker RESOLUTION
      (let ((cb (near/promise_batch_then ol (near/current_account_id))))
        (near/promise_batch_action_function_call cb "post_result"
          (str-cat "{\"caller\":\"" caller "\",\"ts\":\"" ts "\",\"text\":\"" text "\"}")
          "0" GAS-CB)
        (near/storage_set "last-post" (str-cat caller "/" ts))
        (near/promise_return cb))))))))

(define (post_result)
  (begin
    (let ((caller (near/json_get_str "caller"))
          (ts     (near/json_get_str "ts"))
          (text   (near/json_get_str "text"))
          (ok  (near/promise_succeeded 0)))
      (if (= ok 0)
          (near/storage_set "last" (str-cat caller "|EXEC-FAIL"))
          (let ((out (near/promise_result 0)))
            ;; stdout arrives DOUBLE-SERIALIZED: a JSON string wrapping the JSON.
            ;; Unwrap: strip outer quotes, unescape \" pairs.
            (let ((inner (str-replace (str-slice out 1 (- (str-length out) 1))
                                      "\\\"" "\"")))
            (near/storage_set "raw" out)
            (near/log (str-cat "RAWOUT " out))
            (let ((id  (json-get-str "id" inner))
                  (sig (json-get-str "sig" inner))
                  (pk  (json-get-str "pk" inner))
                  (c   (default (near/storage_get "ev-count") "0")))
              (let ((ts-s  (str-slice ts 0 10))
                    (nonce (str-slice ts 13 19)))
                ;; full NIP-01 event, relay-ready (text is quote-free by guard)
                (near/storage_set (str-cat "ev:" c)
                  (str-cat "{\"id\":\"" id "\",\"pubkey\":\"" pk
                    "\",\"created_at\":" ts-s ",\"kind\":1,"
                    "\"tags\":[[\"t\",\"crosspost\"],[\"nonce\",\"" nonce "\"]],"
                    "\"content\":\"" text "\",\"sig\":\"" sig "\"}"))
                (near/storage_set (str-cat "evn:" c) caller)
                (near/storage_set (str-cat "pk:" caller) pk)
                (near/storage_set "ev-count" (to-string (+ (str->num c) 1)))
                (near/storage_set "last" (str-cat caller "|OK|" pk))
                (near/log (str-cat "EVENT crosspost " caller " pk " pk)))))))
    0)))

;; ── views ──────────────────────────────────────────────────────────
(define (last)      (near/return_str (default (near/storage_get "last") "")))
(define (last-post) (near/return_str (default (near/storage_get "last-post") "")))

(define (get_count) (near/json_return_str (default (near/storage_get "ev-count") "0")))

(define (get_pk)
  (near/json_return_str (default (near/storage_get (str-cat "pk:" (near/json_get_str "caller"))) "")))

;; get_outbox {"from":"<n>"} → JSON array of events from n (exclusive) to count
(define (out-walk i end acc)
  (if (= i end)
      acc
      (out-walk (+ i 1) end
        (str-cat acc (str-cat (if (= i 0) "" ",")
                              (default (near/storage_get (str-cat "ev:" (to-string i))) "null"))))))
(define (get_outbox)
  (let ((from (str->num (near/json_get_str "from")))
        (cnt  (str->num (default (near/storage_get "ev-count") "0"))))
    (near/return_str (str-cat "[" (out-walk from cnt "") "]"))))

(export "post" post #f)
(export "post_result" post_result #f)
(export "last" last #t)
(export "last-post" last-post #t)
(export "get_count" get_count #t)
(export "get_pk" get_pk #t)
(export "get_outbox" get_outbox #t)
