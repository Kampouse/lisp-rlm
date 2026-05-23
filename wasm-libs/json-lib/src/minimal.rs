// Minimal JSON key extractor + string concat — emitted directly as WASM instructions
// No external crates needed. ~200 bytes of WASM.
//
// json_get(json_ptr, json_len, key_ptr, key_len) -> i64  (packed ptr<<32 | len)
// str_concat(a_ptr, a_len, b_ptr, b_len) -> i64  (packed ptr<<32 | len)
//
// Both use a bump-allocated output area starting at OUT_AREA (0x20000)
