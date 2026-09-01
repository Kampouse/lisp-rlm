;; wallet_factory_full.lisp — the full wallet factory surface for the
;; diff harness: per-method exports 1:1 with the Rust SDK shape, plus
;; the essential NEAR imports (storage_write/read, predecessor_account_id,
;; sha256 via set-wallet-code's hash). Compile-structure test only —
;; the behavioral diff runs against wallet_factory.lisp.
(define (init)
  (begin
    (near/storage_set "owner" (near/predecessor_account_id))
    (near/storage_set "code_hash" "")
    (near/store "code_size" 0)
    0))

(define (migrate)
  (begin
    (near/store "code_size" (near/load "code_size"))
    0))

(define (get-code-hash)
  (default (near/storage_get "code_hash") ""))

(define (get-wallet-code-size)
  (near/load "code_size"))

(define (set-wallet-code size)
  (if (= (near/predecessor_account_id) (default (near/storage_get "owner") ""))
    (begin
      (near/storage_set "code_hash" (hex-encode (near/sha256 "wallet-code")))
      (near/store "code_size" size)
      0)
    (begin (near/abort "owner only") 0)))

(define (create-wallet name)
  (if (> (str-len name) 0)
    (begin (near/storage_set (str-cat "w:" name) "1") 0)
    (begin (near/abort "bad name") 0)))

(export "init" init true)
(export "migrate" migrate true)
(export "get_code_hash" get-code-hash true)
(export "get_wallet_code_size" get-wallet-code-size true)
(export "set_wallet_code" set-wallet-code false)
(export "create_wallet" create-wallet false)
