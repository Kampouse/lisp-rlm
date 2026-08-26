;; T12 — int/float coercion and mixed-type comparison matrix
;;
;; Ground rules: (2) every constant below verified with python3.
;; Semantics pinned: numeric comparisons and arithmetic PROMOTE to float when
;; any operand is float; integer / truncates toward zero; mod is euclidean;
;; to-int truncates toward zero; floats print via Rust {} (5.0 prints "5",
;; 2.5 prints "2.5") — trailing ".0" is dropped, this is display semantics.
;; (to-float "3.5") parses strings since commit e35c7c9 (was silent 0.0).
;; KNOWN (GAPS.md): (to-int "abc") → 0 silently on the compiled path (the
;; shadowed dispatch implementation errors instead — lenient-vs-strict
;; divergence between the two builtin tables).

(println (= 1 1.0))        ; true  — mixed compare promotes to float
(println (= 2 2.0))        ; true
(println (< 1 1.5))        ; true
(println (< 2 2.5))        ; true
(println (> 2.5 2))        ; true
(println (+ 1 2.5))        ; 3.5
(println (- 5 7.5))        ; -2.5
(println (* 2 2.5))        ; 5    (float result prints without .0)
(println (/ 7 2))          ; 3    — int/int truncates
(println (/ 7.0 2))        ; 3.5  — any float promotes
(println (/ 7 2.0))        ; 3.5
(println (/ -7 2))         ; -3   — truncation toward zero (C-style)
(println (mod -7 2))       ; 1    — euclidean remainder
(println (min 1 2.5))      ; 1
(println (max 2.5 2))      ; 2.5
(println (to-float 5))     ; 5    — Float(5.0) prints as "5"
(println (to-float "3.5")) ; 3.5  — string parse (fixed; was 0)
(println (to-int 2.9))     ; 2    — truncation toward zero
(println (to-int -2.9))    ; -2
(println (to-int "42"))    ; 42
(println (to-string 2.5))  ; "2.5"
(println (to-string 4.0))  ; "4"
(println (= 0.0 0))        ; true
