#!/usr/bin/env python3
"""Split near_mock.rs into a directory module: gas.rs, state.rs, promises.rs,
hosts.rs, mod.rs. Mechanical extraction on exact top-level item boundaries;
fail loudly if any anchor is missing."""

import re, os

SRC = "/Users/asil/dev/lisp-rlm/src/bin/near_mock.rs"
DST = "/Users/asil/dev/lisp-rlm/src/bin/near_mock"
text = open(SRC).read()
lines = text.split("\n")

# Locate top-level items by line-scanning: an item starts at a line matching
# ^(fn|pub fn|struct|enum|const|static|impl|mod|thread_local!|macro_rules!|#\[derive|type|use) at column 0.
# We'll collect (start_line, end_line, header) for items we want to move.

def find_item(header_re, occurrence=1):
    """Find top-level item start (0-based line idx) whose header matches, then
    its end by brace counting from the header line (handles derives/attrs)."""
    pat = re.compile(header_re)
    count = 0
    i = 0
    while i < len(lines):
        m = pat.match(lines[i])
        if m:
            count += 1
            if count == occurrence:
                # include preceding contiguous attr/derive/comment lines
                start = i
                j = i - 1
                while j >= 0 and (lines[j].startswith("//") or lines[j].startswith("#[") or lines[j].startswith("#![")):
                    start = j
                    j -= 1
                # brace count from the item's first '{' (or the header line if it has one)
                depth = 0
                started = False
                k = i
                while k < len(lines):
                    for ch in lines[k]:
                        if ch == "{":
                            depth += 1
                            started = True
                        elif ch == "}":
                            depth -= 1
                    if started and depth == 0:
                        return start, k
                    k += 1
                raise RuntimeError(f"unbalanced item at line {i}: {lines[i]}")
        i += 1
    raise RuntimeError(f"header not found: {header_re}")

# name -> (header_regex, occurrence)
ITEMS = {
    "state.rs": [
        (r"^struct MockState", 1),
        (r"^impl MockState", 1),
        (r"^fn state_file", 1),
        (r"^fn prefixed_key", 1),
        (r"^fn write_reg_checked", 1),
        (r"^fn snapshot_partition", 1),
        (r"^fn restore_partition", 1),
    ],
    "gas.rs": [
        (r"^#\[derive\([^\)]*\)\]\s*$", 1),  # placeholder, replaced below
    ],
}

# Gas schedule struct: find by field anchor instead
def find_item_containing(anchor, back=30, fwd=40):
    for i, l in enumerate(lines):
        if anchor in l:
            start = max(0, i - back)
            # walk back to item start: first line matching ^(#\[derive|pub struct|struct)
            j = i
            while j > 0:
                if re.match(r"^(#\[derive|pub struct|struct|pub fn|fn|pub const|const)", lines[j]):
                    start = j
                    # include attrs above
                    k = j - 1
                    while k >= 0 and (lines[k].startswith("//") or lines[k].startswith("#[")):
                        start = k
                        k -= 1
                    break
                j -= 1
            # brace count
            depth = 0
            started = False
            k2 = start
            while k2 < len(lines):
                for ch in lines[k2]:
                    if ch == "{":
                        depth += 1
                        started = True
                    elif ch == "}":
                        depth -= 1
                if started and depth == 0:
                    return start, k2
                k2 += 1
    raise RuntimeError(f"anchor not found: {anchor}")

MOVE = {
    "state.rs": [
        find_item(r"^struct MockState"),
        find_item(r"^fn state_file"),
        find_item(r"^fn prefixed_key"),
        find_item(r"^fn write_reg_checked"),
        find_item(r"^fn snapshot_partition"),
        find_item(r"^fn restore_partition"),
    ],
    "gas.rs": [
        find_item(r"^struct GasSchedule"),
        find_item(r"^impl GasSchedule"),
        find_item(r"^impl Default for GasSchedule"),
        find_item(r"^const STAKING_COST_PER_BYTE"),
        find_item(r"^fn apply_staking_delta"),
        find_item(r"^fn locked_balance_for"),
        find_item(r"^fn trie_charge"),
        find_item(r"^fn trie_charge_write"),
        find_item(r"^fn splitmix64"),
        find_item(r"^fn stub_warn"),
    ],
    "promises.rs": [
        find_item(r"^enum PAction"),
        find_item(r"^struct PromiseBatch"),
        find_item(r"^fn dag_push"),
        find_item(r"^fn sub_execute"),
        find_item(r"^fn execute_promise"),
    ],
    "hosts.rs": [
        find_item(r"^fn build_env_linker"),
    ],
}

# EXEC_CTX / EXEC_CFG thread_locals + mock_cfg + exec_ctx_or_default + safe_report
# stay in mod.rs (referenced by everything).

# Flatten, sort by start, ensure no overlaps
all_spans = []
for mod, spans in MOVE.items():
    for s in spans:
        all_spans.append((s[0], s[1], mod))
all_spans.sort()
for a, b in zip(all_spans, all_spans[1:]):
    assert a[1] < b[0], f"overlap: {a} vs {b}"

os.makedirs(DST, exist_ok=True)
moved = {m: [] for m in MOVE}
taken = set()
for s, e, mod in all_spans:
    for ln in range(s, e + 1):
        taken.add(ln)
    moved[mod].append("\n".join(lines[s:e + 1]))

def pubify(src_text):
    """Make top-level items pub(crate) and add module doc header."""
    out = re.sub(r"^(fn |struct |enum |const |static |impl |type )", r"pub(crate) \1", src_text, flags=re.M)
    return out

HEADERS = {
    "state.rs": "//! MockState storage model: key/value map + registers, partition\n//! snapshot/restore for failed-receipt revert, register limits.",
    "gas.rs": "//! Gas fee schedule (loadable via --gas-schedule), storage-staking\n//! accounting, trie charging, stub warnings.",
    "promises.rs": "//! Promise DAG: batches, sub-execution, transfer/fn-call settlement.",
    "hosts.rs": "//! build_env_linker: all 92 NEAR host functions (storage, registers,\n//! context, crypto, promises, precompiles).",
}
for mod, chunks in moved.items():
    body = "\n\n".join(c for c in chunks)
    body = pubify(body)
    prelude = "\n".join(l for l in lines if l.startswith("use ") or l.startswith("pub use "))
    with open(os.path.join(DST, mod), "w") as f:
        f.write(HEADERS[mod] + "\n\n" + prelude + "\n\n" + body + "\n")

# mod.rs = remainder
remaining = [l for i, l in enumerate(lines) if i not in taken]
open(os.path.join(DST, "mod.rs"), "w").write("\n".join(remaining))

print("split complete:")
for mod, chunks in moved.items():
    print(f"  {mod}: {len(chunks)} items")
rem = "\n".join(remaining)
print(f"  mod.rs: {len(remaining)} lines (from {len(lines)})")
