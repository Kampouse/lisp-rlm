(module
  ;; Minimal NEAR oracle stub: get_fresh → utf8 price string, stale_check → "STALE!"
  ;; value_return(len, ptr) — NOT (ptr, len); the mock's host takes length first.
  (import "env" "value_return" (func $value_return (param i64 i64)))
  (import "env" "input" (func $input (param i64)))
  (memory (export "memory") 1)
  (data (i32.const 100) "12345.6")
  (data (i32.const 200) "STALE!")
  (func (export "get_fresh")
    ;; consume input register id (args readable but ignored)
    (call $input (i64.const 0))
    (call $value_return (i64.const 7) (i64.const 100)))
  (func (export "stale_check")
    (call $value_return (i64.const 6) (i64.const 200))))
