;; T13 — string builtin boundary matrix (empty, single-char, out-of-range)
;;
;; Ground rules: (1) empty-string identity is NOT special-cased anywhere;
;; (2) constants verified with python3. Range errors must be REAL errors
;; (exit 1) — since commit f79d26c they report "str-substring: indices out
;; of range (...)" instead of the misclassified "unknown builtin".
;; KNOWN divergences, documented in GAPS.md (torture r2):
;;  - str-length counts BYTES, not chars: (str-length "héllo") → 6.
;;    (The shadowed dispatch_strings impl counts chars — dead code.)
;;  - str-split does NOT filter empty parts: (str-split "" ",") → ("")
;;    and "a,,b" → ("a" "" "b"). (dispatch impl filters — dead code.)

(println (str-length ""))        ; 0
(println (str-length "abc"))     ; 3
(println (str-length "héllo"))   ; 6  — KNOWN: byte length, é is 2 bytes
(println (str-length "x"))       ; 1
(println (str-cat "" "x"))       ; "x" — empty identity
(println (str-cat "x" ""))       ; "x"
(println (str-cat "" ""))        ; ""
(println (str-cat "ab" "cd" "ef")) ; "abcdef"
(println (str-substring "abc" 0 3)) ; "abc" — full range
(println (str-substring "abc" 3 3)) ; ""   — tail empty slice is in range
(println (str-substring "abc" 1 2)) ; "b"
(println (str-trim "   "))       ; ""
(println (str-trim "  hi  "))    ; "hi"
(println (str-index-of "abc" "z")) ; -1 — miss
(println (str-index-of "abc" "b")) ; 1
(println (str-split "a,b" ","))  ; ("a" "b")
(println (str-split "" ","))     ; ("") — KNOWN: unfiltered empty part
(println (str-split "a,,b" ",")) ; ("a" "" "b")
(println (str-contains "" ""))   ; true
(println (str-contains "abc" "b")) ; true
(println (str-contains "abc" "z")) ; false
(println (str-upcase "aBc"))     ; "ABC"
(println (str-downcase "aBc"))   ; "abc"
(println (str-replace "aaa" "a" "b")) ; "bbb"
(println (str= "" ""))           ; true
(println (str= "" "x"))          ; false
(println (str!= "" "x"))         ; true
(println (str-starts-with "abc" "ab")) ; true
(println (str-starts-with "" ""))      ; true
(println (str-ends-with "abc" "bc"))   ; true
(println (str-ends-with "" ""))        ; true
