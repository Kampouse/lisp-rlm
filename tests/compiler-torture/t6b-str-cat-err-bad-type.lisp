;; error: non-string arg to str-cat (strings-only builtin — wasm untag assumes
;; TAG_STR; a Num arg is a hard error in the interpreter, silent mis-read on wasm)
(println (str-cat "x" 42))
