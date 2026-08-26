# P2 Memory Model Verification

This directory contains F* specifications for the WASI P2 bridge memory model.

## Memory Layout

```
Address      Size      Purpose
--------     ----      -------
0x0000       32KB      STDIN_BUF (input buffer)
0x8000       32KB      Reserved
0x18000      4 bytes   STDIN_LEN (length of stdin data)
...                     ...
0x1F000      16 bytes  RET_AREA (canonical ABI result)
0x20000      ...       HEAP_START (cabi_realloc bump pointer)
```

## Verification

Run `make verify` to check:
1. Memory regions are disjoint (no overlap)
2. RET_AREA is before HEAP_START (safe for canonical ABI)
3. STDIN_BUF can hold maximum stdin size
4. Bridge copy cannot corrupt heap

## The Bug We're Preventing

The F* spec proves:
- `memory_regions_disjoint` - STDIN_BUF, STDIN_LEN, RET_AREA, HEAP don't overlap
- `bridge_copy_is_safe` - Copy from heap to STDIN_BUF cannot corrupt heap
- `blocking_read_no_stdin_corruption` - blocking_read writes to heap, not STDIN_BUF

## Current Issue

The current bug is that `blocking_read` returns data but the bridge reads null bytes.
This could be:
1. Memory not shared correctly between modules (bridge uses env.memory but canon-lower uses memory 0)
2. cabi_realloc not called or returns wrong pointer
3. Canonical ABI result layout differs from assumed (discriminant, ptr, len)

## Running

```bash
# Verify memory layout
fstar P2Memory.fst --odir out

# Check specific theorem
fstar P2Memory.fst --query 'memory_regions_disjoint'
```