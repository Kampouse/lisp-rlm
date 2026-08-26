;; error: out-of-range str-substring must be a hard error with the real
;; range message. Since commit f79d26c the message is
;; "str-substring: indices out of range (0..5 for len 3)" — before the fix
;; this same call misreported as "unknown builtin 'str-substring'" because
;; eval_builtin swallowed dispatch-module errors.
(println (str-substring "abc" 0 5))
