# Plan: Native WASI P2 HTTP for Lisp-RLM

## Current State (Working)
- `(http/get url)` → component composition with Rust HTTP provider (72KB)
- 71K instructions, 274ms on layerd
- Uses `wasi_snapshot_preview1` adapter (48K) + Rust runtime init (23K)

## Goal
- **<5K instructions** for `(http/get url)`
- No preview1 adapter, no Rust provider
- Production OutLayer compatible (wasi:http@0.2.2 native)
- ~2KB core WASM

## Architecture

### Phase 1: Direct WASI 0.2.2 Stdin/Stdout (saves ~48K)
Replace `wasi_snapshot_preview1.fd_read/fd_write` + 19KB adapter with direct calls:
- Import `wasi:cli/stdin@0.2.2` `get-stdin` → `() -> i32` (handle)
- Import `wasi:io/streams@0.2.2` `blocking-read` → `(i32, i64, i32)` (stream, len, ret_ptr)
- Import `wasi:cli/stdout@0.2.2` `get-stdout` → `() -> i32` (handle)
- Import `wasi:io/streams@0.2.2` `blocking-write-and-flush` → `(i32, i32, i32)` (stream, ptr, len)
- Import resource-drop for input-stream, output-stream

**Steps:**
1. Add new import category `WASI_P2` (base 0xFF05_0000)
2. Add `(wasi-p2/read-stdin)` and `(wasi-p2/write-stdout)` builtins
3. In P2 build, skip preview1 adapter — emit native 0.2.2 imports
4. Update WIT to import wasi:cli and wasi:io directly
5. Test echo program on layerd (should be ~500 instructions)

**Estimate: ~200 lines of emitter code**

### Phase 2: Direct wasi:http GET (saves ~23K)
Emit `wasi:http/outgoing-handler@0.2.2` calls from Lisp instructions:

**Required imports (all flat i32 params):**
```
0  [constructor]fields                           () -> i32
1  [constructor]outgoing-request                 (i32) -> i32
2  [method]outgoing-request.set-scheme           (i32, i32, i32, i32, i32) -> i32
3  [method]outgoing-request.set-authority        (i32, i32, i32, i32) -> i32
4  [method]outgoing-request.set-path-with-query  (i32, i32, i32, i32) -> i32
5  [method]outgoing-request.body                 (i32, i32)
6  [static]outgoing-body.finish                  (i32, i32, i32, i32)
7  [outgoing-handler]handle                      (i32, i32, i32, i32)
8  [method]future-incoming-response.subscribe    (i32) -> i32
9  [method]pollable.block                        (i32)
10 [method]future-incoming-response.get          (i32, i32)
11 [method]incoming-response.consume             (i32, i32)
12 [method]incoming-body.stream                  (i32, i32)
13 [method]input-stream.blocking-read            (i32, i64, i32)
14-23 [resource-drop] × 10                       (i32)
```

**URL parsing at runtime (~30 instructions):**
- Scan bytes for `://` (skip scheme)
- Scan for `/` (split authority / path)
- Default path to `/` if none

**HTTP flow (~150 instructions):**
1. `fields.new()` → f
2. `outgoing-request.new(f)` → req
3. `req.set-scheme(HTTPS)` 
4. `req.set-authority(host_ptr, host_len)`
5. `req.set-path-with-query(path_ptr, path_len)`
6. `req.body(ret)` → body
7. `outgoing-body.finish(body, empty_fields)`
8. `outgoing-handler.handle(req, options, ret)` → future
9. `future.subscribe()` → pollable
10. `pollable.block()`
11. `future.get(ret)` → response
12. `response.consume(ret)` → in_body
13. `incoming-body.stream(in_body, ret)` → stream
14. `stream.blocking-read(stream, 65536, ret)` → body bytes
15. Drop all handles
16. Return body as packed string

**Canonical ABI notes:**
- `option<http-scheme>`: tag=0 (some), discriminant=1 (HTTPS), 0, 0
- `result<T, error>`: ret_ptr points to [discrim i32, ok_val..., err_val...]
- Need to read ok_val from ret_ptr+4 for handles

**Steps:**
1. Add WASI_HTTP import category (base 0xFF06_0000) 
2. Define all 24 function signatures
3. Add `http/get` builtin that emits the full sequence
4. URL parse: byte scan loop for `://` and `/`
5. Handle lifecycle: create → use → drop
6. Response body: blocking-read into buffer, return packed string
7. Update WIT to import wasi:http/types and wasi:http/outgoing-handler
8. Test on layerd with wasmtime first

**Estimate: ~400 lines of emitter code**

### Phase 3: http/post
Same pattern but with body writing through outgoing-body stream.

## Testing Strategy
1. Build minimal test: just stdin/stdout echo → verify <1K instructions
2. Build HTTP test: fetch httpbin.org → verify response
3. Run on layerd → compare instruction count
4. Run on wasmtime with wasi-http → verify compatibility

## Reference Files
- Rust HTTP provider: `/tmp/http-provider/src/lib.rs` (working reference)
- Core module imports: captured in chat history (exact signatures)
- WASI P2 WIT: `~/.cargo/registry/src/.../wasmtime-wasi-http-28.0.1/wit/deps/http/`

## Current Blockers

### Phase 1 Blocked: wasi:cli/run export required
- Layerd instantiates components via `wasi:cli/run@0.2.2` export
- The preview1 adapter converts core `_start` → `wasi:cli/run.run`
- Without the adapter, we need to emit `wasi:cli/run` as a component export
- **Options:**
  A. Modify layerd worker to accept `_start` without `wasi:cli/run` wrapper
  B. Build our own minimal "command adapter" that just wraps `_start` → `wasi:cli/run`
  C. Emit the component export directly using wit-component encoding

### Phase 2 Partially Working
- wasi_p2_native.rs emits 2KB core WASM with 24 native wasi:http imports
- Validates clean, embeds with custom WIT, produces 11KB component
- Cannot run on layerd due to wasi:cli/run blocker above
- Canonical ABI details need runtime verification (authority/path params, result discriminants)

### Realistic Next Step
The adapter tax (48K instructions) is unavoidable without worker changes.
Focus on optimizing the HTTP provider instead:
- Current Rust provider: 443KB, ~23K instructions
- Could build a C provider using raw wasi:http imports → ~5KB, ~1K instructions
- Or strip Rust provider deps (no url crate, no icu_normalizer)

## Execution Order
1. Phase 1 first (stdin/stdout) — biggest win, simpler
2. Verify <1K instructions on layerd
3. Phase 2 (HTTP GET) — more complex but well-documented
4. Verify <5K total
5. Phase 3 (POST) — straightforward extension
