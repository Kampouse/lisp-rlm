;; wallet_factory.lisp — minimal wallet factory for the diff harness.
;; _run choreography: init (stores owner from predecessor_account_id,
;; empty code_hash, code_size 0) then set_wallet_code(100). The harness
;; (tests/test_wallet_diff.rs) runs `_run` under a mocked NEAR host and
;; asserts storage: `owner` non-empty, `code_size` = tagged i64 100.
(define (init)
  (begin
    (near/storage_set "owner" (near/predecessor_account_id))
    (near/storage_set "code_hash" "")
    (near/store "code_size" 0)
    0))

(define (set-wallet-code size)
  (if (= (near/predecessor_account_id) (default (near/storage_get "owner") ""))
    (begin (near/store "code_size" size) 0)
    (begin (near/abort "owner only") 0)))

(define (run)
  (begin (init) (set-wallet-code 100)))
