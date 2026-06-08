;; Test: call cabi_realloc(0, 0, 1, 100) to allocate 100 bytes
;; Then write "hello" there and return it as a string
(define (run)
  ;; We can't call cabi_realloc directly from Lisp
  ;; But we can test: does rpc-view work with the Rust WASM?
  (rpc-view "system.near" "get_account_info" "{}" ""))
