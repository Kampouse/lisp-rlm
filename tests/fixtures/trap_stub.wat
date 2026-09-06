(module
  ;; trap-semantics probe fixture (scenario-runner meta-tests + fuzz):
  ;;   write_a/write_b store ka=a / kb=b; `spin` stores nothing but burns
  ;;   fuel forever — under a per-step gas cap it traps instantly, and the
  ;;   runner's NEAR-atomicity rollback must leave storage untouched.
  ;; Used by tests/test_scenario_meta.rs and tests/test_scenario_fuzz.rs
  ;; to pin expect/contains/trap/gas/snapshot/restore verdict semantics.
  (import "env" "value_return" (func $value_return (param i64 i64)))
  (import "env" "storage_write" (func $storage_write (param i64 i64 i64 i64 i64) (result i64)))
  (memory (export "memory") 1)
  (data (i32.const 300) "ka")
  (data (i32.const 400) "a")
  (data (i32.const 310) "kb")
  (data (i32.const 410) "b")
  (func (export "write_a")
    (drop (call $storage_write (i64.const 2) (i64.const 300) (i64.const 1) (i64.const 400) (i64.const 0)))
    (call $value_return (i64.const 2) (i64.const 400)))
  (func (export "write_b")
    (drop (call $storage_write (i64.const 2) (i64.const 310) (i64.const 1) (i64.const 410) (i64.const 0)))
    (call $value_return (i64.const 1) (i64.const 410)))
  (func (export "spin")
    (loop $l (br $l))))
