//! Tagged-value runtime WAT fragment.
//!
//! Value = i64: [tag:8 bits | payload:56 bits]
//!   tag 0 = number (signed i56)
//!   tag 1 = bool (0 or 1)
//!   tag 2 = nil
//!   tag 3 = cons ptr (i32 address)
//!   tag 4 = string ptr (i32 address)
//!
//! Encoding: value = (tag as i64 << 56) | (payload as u64 & 0x00FFFFFFFFFFFFFF)
//! Decoding: tag = (value >> 56) as u8, payload = value & 0x00FF...

pub fn runtime_functions() -> &'static str {
    r#"
  ;; ── Bump allocator ──
  (func $rt_alloc (param $size i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (i32.load (i32.const 0)))
    (i32.store (i32.const 0) (i32.add (local.get $ptr) (local.get $size)))
    (local.get $ptr)
  )
  (func $rt_init
    (i32.store (i32.const 0) (i32.const 1024))
  )

  ;; ── Value encoding ──
  ;; make_num(n) = (0 << 56) | (n & mask)
  ;; make_bool(b) = (1 << 56) | b
  ;; make_nil() = (2 << 56)
  ;; make_cons(ptr) = (3 << 56) | ptr
  ;; make_str(ptr) = (4 << 56) | ptr
  ;; tag(v) = v >> 56
  ;; payload(v) = v & 0x00FF...

  (func $make_num (param $n i64) (result i64)
    (i64.and (local.get $n) (i64.const 0x00FFFFFFFFFFFFFF))
  )
  (func $make_bool (param $b i32) (result i64)
    (i64.or (i64.const 72057594037927936) (i64.extend_i32_u (local.get $b)))
  )
  (func $make_nil (result i64) (i64.const 144115188075855872))
  (func $make_cons (param $ptr i32) (result i64)
    (i64.or (i64.const 216172782113783808) (i64.extend_i32_u (local.get $ptr)))
  )
  (func $make_str (param $ptr i32) (result i64)
    (i64.or (i64.const 288230376151711744) (i64.extend_i32_u (local.get $ptr)))
  )

  (func $tag_of (param $v i64) (result i32)
    (i32.wrap_i64 (i64.shr_u (local.get $v) (i64.const 56)))
  )
  (func $payload_of (param $v i64) (result i64)
    (i64.and (local.get $v) (i64.const 0x00FFFFFFFFFFFFFF))
  )
  (func $ptr_of (param $v i64) (result i32)
    (i32.wrap_i64 (i64.and (local.get $v) (i64.const 0x00FFFFFFFFFFFFFF)))
  )

  ;; ── Cons cells ──
  ;; Layout: [car_tag:i32][car_payload:i64][cdr_tag:i32][cdr_payload:i64] = 24 bytes
  ;; We store full i64 values directly (tag+payload packed)

  (func $cons (param $car i64) (param $cdr i64) (result i64)
    (local $p i32)
    (local.set $p (call $rt_alloc (i32.const 16)))
    (i64.store (local.get $p) (local.get $car))
    (i64.store offset=8 (local.get $p) (local.get $cdr))
    (call $make_cons (local.get $p))
  )
  (func $car (param $v i64) (result i64)
    (i64.load (call $ptr_of (local.get $v)))
  )
  (func $cdr (param $v i64) (result i64)
    (i64.load offset=8 (call $ptr_of (local.get $v)))
  )

  ;; ── Type checks (return i32 bool) ──
  (func $is_num (param $v i64) (result i32) (i32.eqz (call $tag_of (local.get $v))))
  (func $is_bool (param $v i64) (result i32) (i32.eq (call $tag_of (local.get $v)) (i32.const 1)))
  (func $is_nil (param $v i64) (result i32) (i32.eq (call $tag_of (local.get $v)) (i32.const 2)))
  (func $is_cons (param $v i64) (result i32) (i32.eq (call $tag_of (local.get $v)) (i32.const 3)))
  (func $is_str (param $v i64) (result i32) (i32.eq (call $tag_of (local.get $v)) (i32.const 4)))

  ;; ── Truthiness ──
  (func $is_truthy (param $v i64) (result i32)
    (local $t i32)
    (local.set $t (call $tag_of (local.get $v)))
    ;; nil is falsy
    (if (result i32) (i32.eq (local.get $t) (i32.const 2)) (then (i32.const 0))
    (else
      ;; bool: value of payload
      (if (result i32) (i32.eq (local.get $t) (i32.const 1))
        (then (i32.wrap_i64 (call $payload_of (local.get $v))))
        (else (i32.const 1)) ;; everything else is truthy
      )
    ))
  )
"#
}
