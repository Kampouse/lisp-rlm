# stdin Bug in lisp-rlm P2 Canonical ABI

## Issue
The `(json-get-str "key")` form (which reads from stdin via `fd_read`) fails with corrupted data.
The `blocking_read` canonical ABI lowering writes `Result<List<u8>>` to `RET_AREA`, but the
pointer field (`RET_AREA[4:8]`) contains an invalid address.

## Root Cause
The canonical ABI for `blocking-read: func(len: u64) -> result<list<u8>, stream-error>`:
1. Host returns `Result<List<u8>>`
2. Canonical ABI allocates via `cabi_realloc` (bump allocator starting at 128 MB)
3. Copies data to allocated buffer
4. Writes `(discriminant=0, ptr, len)` to `ret_area`

The bridge sees:
- `RET_AREA[0:4]` = 0 (OK discriminant) ✓
- `RET_AREA[4:8]` = garbage pointer ✗
- `RET_AREA[8:12]` = 72 (wrong length) ✗

Expected:
- `RET_AREA[4:8]` = ~134217728 (pointer from `cabi_realloc`)
- `RET_AREA[8:12]` = 26 (actual input length)

## Binary Verification
The binary encoding is correct:
- `0x12 0x00` = `(core instance 0)` (memory module with `cabi_realloc`)
- Bridge imports memory from `env`, satisfied by memory module
- `canon lower` uses `(memory 0) (realloc 0)` correctly

The issue is in wasmtime's canonical ABI implementation.

## Workaround
Use the 2-argument form `(json-get-str "key" input)` which scans directly from the input buffer:

\`\`\`lisp
;; BROKEN (reads stdin via fd_read)
(define (run)
  (let* ((acct (json-get-str "account_id")))
    (str-cat "{\"received\":\"" acct "\"}")))

;; WORKING (scans input buffer directly)
(define (run input)
  (let* ((acct (json-get-str "account_id" input)))
    (str-cat "{\"received\":\"" acct "\"}")))
\`\`\`

## Test Cases
- `test_input_simple.lisp` - BROKEN (uses stdin)
- `test_input_direct.lisp` - WORKING (uses input param)
- `test_echo.lisp` - WORKING (no stdin)

## Status
- Not fixed (requires wasmtime canonical ABI debugging)
- Working solution available (use input param)
