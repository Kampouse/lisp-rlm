;; error: u128/to-i64 value exceeds i64::MAX (i64::MAX + 1)
(println (u128/to-i64 "9223372036854775808"))
