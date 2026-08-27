;; e32 — scalar string builtins (round 2): upcase/downcase/trim/
;; starts-with/ends-with/replace — previously interp-only, now wasm-emitted.
;; ASCII-only in wasm (interp uses Rust Unicode) — probes stay ASCII.
(define (main)
  ;; case transforms
  (println (str-upcase "hello World 42"))
  (println (str-downcase "HeLLo WORLD 42"))
  ;; trim (both ends, all-ws, no-op, whitespace-only)
  (println (str-trim "  padded  "))
  ;; control-byte case: assert via length (println renders raw control
  ;; bytes differently across surfaces — display divergence, not value)
  (println (str-length (str-trim "\t\nmixed \r ")))
  (println (str-length (str-trim "\t\nmixed \r\v\f ")))
  (println (str-trim "clean"))
  (println (str-length (str-trim "   ")))
  ;; starts/ends with
  (println (str-starts-with "ap:42:alice" "ap:"))
  (println (str-starts-with "tx:42" "ap:"))
  (println (str-starts-with "anything" ""))
  (println (str-ends-with "jemartel.near" ".near"))
  (println (str-ends-with "jemartel.testnet" ".near"))
  (println (str-ends-with "x" ""))
  (println (str-ends-with "ab" "abc"))
  ;; replace: grow, shrink, none, adjacent, overlapping-scan
  (println (str-replace "a-b-c" "-" "+"))
  (println (str-replace "aaaa" "aa" "xxxx"))
  (println (str-replace "aaa" "aa" "b"))
  (println (str-replace "no match here" "zz" "y"))
  (println (str-replace "xx" "x" ""))
  ;; composition: normalize then validate
  (println (str-ends-with (str-downcase (str-trim "  JEMARTEL.NEAR ")) ".near"))
  (println (str-starts-with (str-cat "key:" (str-upcase "ab")) "key:")))
