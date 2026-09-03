#!/usr/bin/env python3
"""board_sum.py — sum cargo test result lines: prints 'battery P/F', exit 1 on any failure."""
import re
import sys

txt = open(sys.argv[1]).read()
p = f = ig = 0
for m in re.finditer(r"test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored", txt):
    p += int(m.group(1))
    f += int(m.group(2))
    ig += int(m.group(3))
print(f"battery {p}/{f} passed/failed ({ig} ignored)")
sys.exit(1 if f else 0)
