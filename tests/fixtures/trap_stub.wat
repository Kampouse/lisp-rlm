(module
  ;; trap-semantics probe fixture (scenario-runner meta-tests + fuzz +
  ;; state-import E2E):
  ;;   write_a/write_b store ka=a / kb=b; `read` returns 1 if ka exists
  ;;   (proves imported state is visible to contract code); `spin` stores
  ;;   nothing but burns fuel forever — under a per-step gas cap it traps
  ;;   instantly, and the runner's NEAR-atomicity rollback must leave
  ;;   storage untouched.
  ;; Used by tests/test_scenario_meta.rs, tests/test_scenario_fuzz.rs and
  ;; the state-import E2E to pin expect/contains/trap/gas/snapshot/restore
  ;; verdict semantics.
  (import "env" "value_return" (func $value_return (param i64 i64)))
  (import "env" "storage_write" (func $storage_write (param i64 i64 i64 i64 i64) (result i64)))
  (import "env" "storage_read" (func $storage_read (param i64 i64 i64) (result i64)))
  (memory (export "memory") 1)
  (data (i32.const 300) "ka")
  (data (i32.const 400) "a")
  (data (i32.const 310) "kb")
  (data (i32.const 410) "b")
  (data (i32.const 500) "yes")
  (data (i32.const 510) "no")
  (func (export "write_a")
    (drop (call $storage_write (i64.const 2) (i64.const 300) (i64.const 1) (i64.const 400) (i64.const 0)))
    (call $value_return (i64.const 2) (i64.const 400)))
  (func (export "write_b")
    (drop (call $storage_write (i64.const 2) (i64.const 310) (i64.const 1) (i64.const 410) (i64.const 0)))
    (call $value_return (i64.const 1) (i64.const 410)))
  ;; read ka -> value_return "yes" if the key exists in state, else "no"
  ;; (near-mock entries take no params and no results)
  (func (export "read")
    (if (i64.eqz (call $storage_read (i64.const 2) (i64.const 300) (i64.const 1)))
      (then (call $value_return (i64.const 2) (i64.const 510)))   ;; "no"
      (else (call $value_return (i64.const 3) (i64.const 500))))) ;; "yes"
  ;; spin: burn fuel forever (never returns)
  (func (export "spin")
    (loop $l (br $l))))
