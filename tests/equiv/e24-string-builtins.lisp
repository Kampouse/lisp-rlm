;; e24 — string builtin surface (all-3-surface: interp ∩ emit ∩ checker = 
;; str-cat, str-concat, str-contains, str-index-of, str-length, str-substring).
;; Emit-only: str-len/slice/repeat/to-num/contains-byte. Interp-only (need
;; wasm emission): upcase/downcase/trim/replace/split*/chunk/join/starts/ends.
;; Full 3-way matrix in corpus/COVERAGE.md §D.
(define (main)
  (println (str-cat "a" "b"))
  (println (str-cat (str-cat "a" "b") "c"))
  (println (str-concat "x" "y"))
  (println (str-length "hello"))
  (println (str-substring "abcdef" 1 3))
  (println (str-contains "hello world" "world"))
  (println (str-contains "hello" "xyz"))
  (println (str-index-of "banana" "na")))
