#!/usr/bin/env python3
"""
P2 Memory Model Verifier

Analyzes WASM P2 component to verify memory layout and identify bugs.
"""

import json
import subprocess
import sys

def analyze_wasm(wasm_path):
    """Analyze WASM component memory layout."""
    
    # Get WASM text format
    result = subprocess.run(
        ['wasm-tools', 'print', wasm_path],
        capture_output=True, text=True
    )
    lines = result.stdout.split('\n')
    
    # Parse memory declarations
    memories = []  # (module_idx, pages)
    for i, line in enumerate(lines):
        if '(memory' in line and 'import' not in line:
            # Example: (memory (;0;) 2048)
            parts = line.strip().split()
            for j, p in enumerate(parts):
                if p == '(memory':
                    pages = int(parts[j+2].rstrip(')'))
                    module_idx = get_module_idx(lines, i)
                    memories.append((module_idx, pages))
    
    # Parse instance wiring
    # Example: (core instance (;0;) (instantiate 1))
    instances = {}
    for i, line in enumerate(lines):
        if '(core instance' in line and '(instantiate' in line:
            # Parse: (core instance (;N;) (instantiate M ...
            inst_idx = int(line.split('(core instance (;')[1].split(';)')[0])
            module_idx = int(line.split('(instantiate ')[1].split(')')[0])
            instances[inst_idx] = module_idx
    
    # Parse memory aliases
    # Example: (alias core export 0 "memory" (core memory (;0;)))
    memory_aliases = {}
    for i, line in enumerate(lines):
        if '(alias core export' in line and '"memory"' in line:
            # Parse: (alias core export INST "memory" (core memory (;N;)))
            inst_idx = int(line.split('export ')[1].split(' ')[0])
            mem_idx = int(line.split('(core memory (;')[1].split(';)')[0])
            memory_aliases[mem_idx] = inst_idx
    
    # Parse canon lower for blocking-read
    # Example: (core func (;13;) (canon lower (func 2) (memory 0) (realloc 0)))
    canon_lowers = {}
    for i, line in enumerate(lines):
        if 'canon lower' in line and 'blocking-read' not in line:
            # Check previous line for func name
            prev = lines[i-1] if i > 0 else ''
            if 'blocking-read' in prev or 'input-stream' in prev.lower():
                # Parse memory and realloc
                if '(memory 0)' in line:
                    canon_lowers['blocking_read'] = {
                        'memory': 0,
                        'realloc': 0 if '(realloc 0)' in line else None
                    }
    
    return {
        'memories': memories,
        'instances': instances,
        'memory_aliases': memory_aliases,
        'canon_lowers': canon_lowers
    }

def get_module_idx(lines, line_idx):
    """Find which core module contains this line."""
    for i in range(line_idx, -1, -1):
        if '(core module (;' in lines[i]:
            return int(lines[i].split('(core module (;')[1].split(';)')[0])
    return -1

def verify_memory_layout():
    """Verify P2 bridge memory constants."""
    
    # Constants from p2_wasi_bridge.rs
    STDIN_BUF = 32768   # 0x8000
    STDIN_LEN = 98304   # 0x18000  
    RET_AREA = 126976   # 0x1F000
    HEAP_START = 131072 # 0x20000
    
    STDIN_BUF_SIZE = 65536  # 64KB
    RET_AREA_SIZE = 16
    
    print("=== Memory Layout Verification ===")
    print()
    print(f"STDIN_BUF:   0x{STDIN_BUF:05X} - 0x{STDIN_BUF + STDIN_BUF_SIZE - 1:05X} ({STDIN_BUF_SIZE} bytes)")
    print(f"STDIN_LEN:   0x{STDIN_LEN:05X} ({4} bytes)")
    print(f"RET_AREA:    0x{RET_AREA:05X} - 0x{RET_AREA + RET_AREA_SIZE - 1:05X} ({RET_AREA_SIZE} bytes)")
    print(f"HEAP_START:  0x{HEAP_START:05X}")
    print()
    
    # Verify non-overlap
    errors = []
    
    # STDIN_BUF must end before STDIN_LEN
    if STDIN_BUF + STDIN_BUF_SIZE > STDIN_LEN:
        errors.append(f"STDIN_BUF overlaps STDIN_LEN!")
    else:
        print(f"✓ STDIN_BUF ends before STDIN_LEN (gap: {STDIN_LEN - STDIN_BUF - STDIN_BUF_SIZE} bytes)")
    
    # STDIN_LEN must end before RET_AREA
    if STDIN_LEN + 4 > RET_AREA:
        errors.append(f"STDIN_LEN overlaps RET_AREA!")
    else:
        print(f"✓ STDIN_LEN ends before RET_AREA (gap: {RET_AREA - STDIN_LEN - 4} bytes)")
    
    # RET_AREA must end before HEAP_START
    if RET_AREA + RET_AREA_SIZE > HEAP_START:
        errors.append(f"RET_AREA overlaps HEAP_START!")
    else:
        print(f"✓ RET_AREA ends before HEAP_START (gap: {HEAP_START - RET_AREA - RET_AREA_SIZE} bytes)")
    
    # RET_AREA must be AFTER STDIN_BUF
    if RET_AREA < STDIN_BUF + STDIN_BUF_SIZE:
        errors.append(f"RET_AREA overlaps STDIN_BUF!")
    else:
        print(f"✓ RET_AREA is after STDIN_BUF")
    
    print()
    
    if errors:
        print("ERRORS:")
        for e in errors:
            print(f"  ✗ {e}")
        return False
    else:
        print("✓ All memory regions are disjoint")
        return True

def verify_blocking_read_layout():
    """Verify canonical ABI result layout for blocking_read."""
    
    print()
    print("=== Canonical ABI Result Layout ===")
    print()
    print("Result<List<u8>, stream-error> for wasm32:")
    print("  RET_AREA[0:4] = discriminant (0=OK, 1=Error)")
    print("  RET_AREA[4:8] = ptr (allocated buffer)")
    print("  RET_AREA[8:12] = len (bytes read)")
    print()
    print("On OK:")
    print("  - discriminant = 0")
    print("  - ptr = result of cabi_realloc(NULL, 0, 1, len)")
    print("  - len = actual bytes read (<= requested len)")
    print("  - buffer at ptr contains the read data")
    print()
    print("On Error:")
    print("  - discriminant = 1")
    print("  - payload contains stream-error variant")
    print()
    
    return True

def main():
    print("P2 Memory Model Verification")
    print("=" * 50)
    print()
    
    # Verify constants
    if not verify_memory_layout():
        sys.exit(1)
    
    # Verify layout
    if not verify_blocking_read_layout():
        sys.exit(1)
    
    # Analyze WASM if provided
    if len(sys.argv) > 1:
        wasm_path = sys.argv[1]
        print(f"Analyzing {wasm_path}...")
        analysis = analyze_wasm(wasm_path)
        print()
        print("=== WASM Analysis ===")
        print(f"Memories: {analysis['memories']}")
        print(f"Instances: {analysis['instances']}")
        print(f"Memory aliases: {analysis['memory_aliases']}")
        print(f"Canon lowers: {analysis['canon_lowers']}")
    
    print()
    print("=== Possible Bug Causes ===")
    print()
    print("If STDIN_BUF contains null bytes:")
    print("  1. cabi_realloc returns garbage (heap not initialized)")
    print("  2. blocking_read writes to wrong memory (memory 0 vs imported)")
    print("  3. Canon-lower result layout differs from assumed")
    print("  4. ptr in RET_AREA[4] points to uninitialized memory")
    print()
    print("Debug steps:")
    print("  1. Print RET_AREA[0:12] after blocking_read")
    print("  2. Print memory at ptr (RET_AREA[4])")
    print("  3. Verify cabi_realloc is called (add debug log)")
    print("  4. Verify wasmtime MemoryInputPipe writes to correct memory")

if __name__ == '__main__':
    main()