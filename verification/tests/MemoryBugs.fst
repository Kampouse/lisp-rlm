(** Memory Model Tests — Demonstrates Bugs Caught by F*

    This module shows how the F* memory model catches bugs that
    would corrupt memory at runtime.

    BUG HISTORY:
    1. commit 557192c: u128/store wrote tagged addresses instead of raw bytes
       - F* would catch: u128_store_tagged uses addr.raw, not addr
       - Theorem u128_roundtrip_tagged proves: store(raw, v) then load(raw) = v
       - If we used addr = raw | tag, bounds check would fail (addr >> raw)

    2. u128_add/sub/mul overflow protection (now proved in Memory.fst)
       - u128_add_safe returns None on overflow
       - u128_sub_safe returns None on underflow
       - u128_mul_safe returns None on overflow or non-positive scalar

    3. i64 checked arithmetic (proved in Memory.fst, tested in I64CheckedTests.fst)
       - i64_add_safe returns None on overflow (same-sign inputs, result flips)
       - i64_sub_safe returns None on underflow (different-sign inputs, result flips)
       - i64_mul_safe returns None on overflow (r/b != a) or MIN*-1
       - Matches Rust checked_add/checked_sub/checked_mul behavior
*)
module MemoryBugs

open LispIR.Memory

// ============================================================
// CONSTANTS FROM LispIR.Memory
// ============================================================

// Shorthand for test readability
let heap_start' = heap_start
let return_buf' = return_buf
let input_buf' = input_buf
let storage_buf' = storage_buf

// ============================================================
// TEST 1: Tagged Pointer Encoding Roundtrip
// ============================================================

// This would pass: encode/decode is bijective for valid tags
let test_tag_roundtrip : bool =
  let payload = 12345 in
  let tag = tag_num in
  let encoded = encode_tag payload tag in
  let decoded_payload = decode_payload encoded in
  let decoded_tag = decode_tag encoded in
  decoded_payload = payload && decoded_tag = tag

// This would pass: all tags 0-7 are valid (including TAG_U128 = 7)
let test_all_tags_valid : bool =
  tag_num <= tag_u128 && 
  tag_bool <= tag_u128 && 
  tag_str <= tag_u128 && 
  tag_array <= tag_u128

// TAG_U128 = TAG_INVALID = 7 (same numeric value, different meaning)
let test_tag_7_is_u128 : bool =
  tag_u128 = 7 && tag_invalid = 7

// ============================================================
// TEST 2: u128 Overflow/Underflow Safety
// ============================================================

// u128_add_safe returns None on overflow
let test_u128_add_overflow : bool =
  let a = { lo = 0x7FFFFFFFFFFFFFFF; hi = 0x7FFFFFFFFFFFFFFF } in
  let b = { lo = 0x7FFFFFFFFFFFFFFF; hi = 0x7FFFFFFFFFFFFFFF } in
  match u128_add_safe a b with
  | None -> true   // Overflow detected
  | Some _ -> false // Would be wrong!

// u128_sub_safe returns None on underflow
let test_u128_sub_underflow : bool =
  let a = { lo = 10; hi = 0 } in
  let b = { lo = 20; hi = 0 } in
  match u128_sub_safe a b with
  | None -> true   // Underflow detected
  | Some _ -> false // Would be wrong!

// u128_mul_safe returns None on overflow or non-positive scalar
let test_u128_mul_overflow : bool =
  let a = { lo = 1000000; hi = 1 } in
  match u128_mul_safe a 1000000 with
  | None -> true   // Overflow detected (hi > 0 && scalar > 1)
  | Some _ -> false

// u128_mul_safe rejects non-positive scalars
let test_u128_mul_nonpositive : bool =
  let a = { lo = 100; hi = 0 } in
  match u128_mul_safe a 0 with
  | None -> true   // Rejected (scalar <= 0)
  | Some _ -> false

// ============================================================
// TEST 3: u128 Bounds Checking
// ============================================================

// Valid address: within WASM memory bounds
let test_u128_bounds_valid : bool =
  u128_bounds_check 0 &&           // Start of memory
  u128_bounds_check 1000 &&        // Middle of memory
  u128_bounds_check (max_memory - 16)  // End of memory (last valid u128)

// Invalid address: beyond memory bounds
let test_u128_bounds_invalid : bool =
  not (u128_bounds_check (-1)) &&              // Negative
  not (u128_bounds_check (max_memory - 15)) && // Only 15 bytes left (need 16)
  not (u128_bounds_check max_memory) &&        // At boundary
  not (u128_bounds_check (max_memory + 1))     // Past end

// ============================================================
// TEST 4: Tagged Address Is Larger Than Raw
// ============================================================

// THE BUG: if we used tagged value as address:
//   addr_tagged = (HEAP_START << 3) | TAG_U128 = HEAP_START * 8 + 7
// This would be 8x larger than the actual address!
let test_tagged_address_is_larger : bool =
  let raw = heap_start in
  let tagged = encode_tag raw tag_u128 in
  // tagged = raw * 8 + 7, which is MUCH larger than raw
  // If we used tagged as memory address, we'd read from wrong location
  tagged > raw

// ============================================================
// TEST 5: Buffer Disjointness
// ============================================================

// F* can prove: fixed buffers don't overlap with heap
let test_buffers_disjoint_from_heap : bool =
  // HEAP_START = 200000
  // RETURN_BUF + 16384 = 49152
  // 200000 > 49152, so heap never overlaps with buffers
  heap_start > return_buf + 16384

// Storage buffer is separate from return buffer
let test_storage_return_disjoint : bool =
  storage_buf + storage_u128_buf < return_buf

// Input buffer is separate from storage
let test_input_storage_disjoint : bool =
  input_buf + 16384 < storage_buf

// ============================================================
// TEST 6: Decode/Encode Inverse Property
// ============================================================

// The lemmas in Memory.fst prove:
// - decode_payload(encode_tag(p, t)) = p for valid tags
// - decode_tag(encode_tag(p, t)) = t for valid tags
// This means encode/decode is a perfect bijection on valid tags

let test_decode_encode_inverse : bool =
  // For any payload and valid tag, decode(encode(p, t)) = p
  let test_case p t =
    t >= 0 && t <= 7 && 
    decode_payload (encode_tag p t) = p &&
    decode_tag (encode_tag p t) = t
  in
  test_case 0 tag_num &&
  test_case 1000 tag_num &&
  test_case 9999 tag_array

// ============================================================
// ALL TESTS
// ============================================================

let all_tests_pass : bool =
  test_tag_roundtrip &&
  test_all_tags_valid &&
  test_tag_7_is_u128 &&
  test_u128_add_overflow &&
  test_u128_sub_underflow &&
  test_u128_mul_overflow &&
  test_u128_mul_nonpositive &&
  test_u128_bounds_valid &&
  test_u128_bounds_invalid &&
  test_tagged_address_is_larger &&
  test_buffers_disjoint_from_heap &&
  test_storage_return_disjoint &&
  test_input_storage_disjoint &&
  test_decode_encode_inverse