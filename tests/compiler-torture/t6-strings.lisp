;; T6 — string builtins in loop context (old str-cat collision class)
;; str-cat is STRINGS-ONLY (matches wasm_emit call_string.rs: untag assumes
;; TAG_STR; Num args are NOT stringified on the wasm path). Convert numbers
;; explicitly with (to-string i) — same name works on both surfaces.
(define (tag i) (str-cat "key-" (to-string i)))
(define (t6 n)
  (let ((out ""))
    (dotimes (i n)
      (set! out (str-cat out (tag i))))
    out))
(println (t6 4))   ; "key-0key-1key-2key-3"
(println (str-cat "x" "42"))  ; "x42"
(println (str-cat "only"))    ; "only" (1-arg identity)
(println (str-cat "a" "b" "c" "d"))  ; "abcd" (variadic)
