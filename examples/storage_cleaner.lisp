;; ═══════════════════════════════════════════════════════════════════
;; examples/storage_cleaner.lisp — storage-iter burn-after-use cleaner
;;
;; Purpose: wipe ALL contract storage keys after a code redeploy left
;; stale state (the gcpool26.testnet reset case). No owner guard — the
;; contract is temporary by design: deploy, clean, delete.
;;
;; SURFACE (new builtins, host fns 36 / 38):
;;   (storage-iter-prefix prefix) → iterator id     [storage_iter_prefix]
;;   (storage-iter-next id)       → key str | nil   [storage_iter_next]
;;   "" prefix = ALL keys. Dash and underscore name forms are both
;;   accepted by the compiler; this file uses dashes. Removal goes
;;   through near/storage_remove (string-safe family) — the bare
;;   storage-remove alias is interpreter-only, it does not lower to wasm.
;;
;; SEMANTICS (interp and wasm agree — keep it that way):
;;   • iteration order is sorted — the interp mock walks a sorted key
;;     snapshot, the NEAR trie host iterates in key order
;;   • removal during iteration is safe on BOTH surfaces: wasm iterates
;;     live trie state, the interp mock skips snapshot keys that were
;;     deleted after the snapshot was taken
;;   • clean() returns the number of keys removed — re-invoke until it
;;     returns 0 (one tx may not clear a very large state)
;;
;; LANDMINES (GAPS.md): numeric 0 is TRUTHY — every exit tests (nil? k)
;; explicitly, never the key itself. No lambdas (T4 closure bug); loop
;; accumulators live in the loop bindings only. count-keys and clear-keys
;; are separate defines (no shared state besides storage itself).
;; ═══════════════════════════════════════════════════════════════════

;; count keys under prefix WITHOUT removing (the "how much is left"
;; probe between clean() invocations)
(define (count-keys prefix)
  (loop ((it (storage-iter-prefix prefix)) (n 0))
    (let ((k (storage-iter-next it)))
      (if (nil? k)
          n
          (recur it (+ n 1))))))

;; wipe every key under prefix; returns number removed
(define (clear-keys prefix)
  (loop ((it (storage-iter-prefix prefix)) (removed 0))
    (let ((k (storage-iter-next it)))
      (if (nil? k)
          removed
          (begin
            (near/storage_remove k)
            (recur it (+ removed 1)))))))

;; every key — "" prefix iterates the whole storage
(define (total-keys) (count-keys ""))

;; ── method shims (deploy surface — JSON in/out like erc20) ──
(define (m-clean)
  (near/json_return_str (to-string (clear-keys ""))))

(define (m-count)
  (near/json_return_str (to-string (count-keys ""))))

(export "clean" m-clean false)
(export "count" m-count true)
