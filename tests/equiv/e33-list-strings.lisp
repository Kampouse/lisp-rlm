;; e33 — list-shaped string builtins (round 3): str-split / str-split-exact /
;; str-chunk / string->list / str-join / list->string. Zero-copy view lists in
;; wasm; ASCII-bounded (COVERAGE.md §A.1).
(define (main)
  ;; split — Rust semantics, empties KEPT (bytecode interp parity)
  (println (len (str-split "a.b..c" ".")))
  (println (car (str-split "a.b..c" ".")))
  (println (car (cdr (str-split "a.b..c" "."))))
  (println (len (str-split "..." ".")))
  (println (len (str-split "abc" "x")))
  (println (car (str-split "abc" "x")))
  (println (len (str-split "" "x")))
  ;; split with multi-char delim (the safe.lisp key grammar!)
  (println (len (str-split "tx:42:lo:hi" ":lo:")))
  (println (car (str-split "tx:42:lo:hi" ":lo:")))
  ;; split-exact — keeps empties
  (println (len (str-split-exact "a..b" ".")))
  (println (car (cdr (str-split-exact "a..b" "."))))
  (println (len (str-split-exact "" "x")))
  ;; chunk — n PIECES (interp port), machine-checked edges
  (println (len (str-chunk "abcdef" 2)))
  (println (car (str-chunk "abcdef" 2)))
  (println (car (cdr (str-chunk "abcdef" 2))))
  (println (len (str-chunk "" 3)))
  (println (len (str-chunk "ab" 1)))
  ;; string->list + list->string round-trip
  (println (len (string->list "abc")))
  (println (car (string->list "abc")))
  (println (list->string (string->list "abc")))
  ;; join — the triple-nested str-cat killer from safe.lisp
  (println (str-join ":" (list "tx" "42" "lo")))
  (println (str-join ":" (list "only")))
  (println (str-join ":" (list)))
  (println (str-join "" (list "a" "b" "c")))
  ;; app-shaped composite: build a safe-style key, split it back out
  (println (str-join ":" (list "tx" "42" "approvals")))
  (println (car (str-split (str-join ":" (list "tx" "42" "approvals")) ":")))
  (println (car (cdr (str-split (str-join ":" (list "tx" "42" "approvals")) ":"))))
  (println (str-ends-with (list->string (string->list "jemartel.near")) ".near")))
