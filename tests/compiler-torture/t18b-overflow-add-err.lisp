;; error: i64 add overflow must be a hard, checked error — never a wrap.
;; Message: "integer overflow in add", exit 1. Same class: (* 2^62 2) →
;; "integer overflow in mul"; (- i64::MIN 1) → "integer overflow in sub".
(println (+ 9223372036854775807 1))
