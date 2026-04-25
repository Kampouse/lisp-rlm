#!/usr/bin/env python3
"""
R7RS Test Runner for lisp-rlm
Strategy: load the whole test file with minimal adaptation, count how many tests pass.
"""

import re, sys, subprocess, os

def main():
    src = '/tmp/chibi-scheme/tests/r7rs-tests.scm'
    if not os.path.exists(src):
        subprocess.run(['git', 'clone', '--depth', '1',
                       'https://github.com/ashinn/chibi-scheme.git',
                       '/tmp/chibi-scheme'], check=True)
    
    with open(src) as f:
        content = f.read()
    
    # Minimal transformations
    content = content.replace('#t', 'true')
    content = content.replace('#f', 'false')
    
    # Remove import lines (multi-line)
    content = re.sub(r'\(import[^)]*\)', '', content, flags=re.DOTALL)
    content = re.sub(r'\(import\s+\([^)]*\)\s*\)', '', content)
    
    # Remove test-begin / test-end
    content = re.sub(r'\(test-end\)', '', content)
    content = re.sub(r'\(test-begin\s+"[^"]*"\)', '', content)
    
    # Define test as a simple function
    header = '''
(define *pass* 0)
(define *fail* 0)

(define (test expected expr)
  (if (= expected expr)
    (set! *pass* (+ *pass* 1))
    (begin
      (set! *fail* (+ *fail* 1))))

(print "R7RS Pass: ") (println *pass*)
(print "R7RS Fail: ") (println *fail*)
'''
    
    # We need test as a macro to avoid evaluating expected
    # But defmacro with quasiquote... let's try a different approach
    # Just wrap each (test E X) in a begin that compares
    
    # Actually, let's define test as a special built-in check
    # lisp-rlm's (test) might not exist. Let's make it a function
    # that compares its two args
    
    # The problem: `expected` gets evaluated too, which is fine for numbers/booleans
    # but for quoted forms like 'a it should work since 'a evaluates to a symbol
    
    output = '/Users/asil/.openclaw/workspace/lisp-rlm/tests/r7rs-conformance.lisp'
    
    with open(output, 'w') as f:
        f.write(';;; R7RS Conformance Tests for lisp-rlm\n')
        f.write(';;; test(a, b) = if a == b then pass, else fail\n\n')
        f.write('(define *pass* 0)\n')
        f.write('(define *fail* 0)\n\n')
        f.write(';; test function - both args evaluated\n')
        f.write('(define (test expected expr)\n')
        f.write('  (if (= expected expr)\n')
        f.write('    (set! *pass* (+ *pass* 1))\n')
        f.write('    (begin\n')
        f.write('      (set! *fail* (+ *fail* 1))\n')
        f.write('      (print "FAIL "))))\n\n')
        f.write(content)
        f.write('\n\n(print "Pass: ") (println *pass*)\n')
        f.write('(print "Fail: ") (println *fail*)\n')
    
    print(f"Written: {output}")
    
    if '--no-run' in sys.argv:
        return
    
    print("Building...")
    r = subprocess.run(['cargo', 'build', '--release'],
                      cwd='/Users/asil/.openclaw/workspace/lisp-rlm',
                      capture_output=True, text=True)
    if r.returncode != 0:
        print("BUILD FAILED:", r.stderr[-500:])
        return
    
    print("Running (60s timeout)...")
    try:
        r = subprocess.run(
            ['./target/release/rlm', output],
            cwd='/Users/asil/.openclaw/workspace/lisp-rlm',
            capture_output=True, text=True, timeout=60
        )
    except subprocess.TimeoutExpired:
        print("TIMEOUT (likely infinite loop in a test)")
        return
    
    # Parse pass/fail from output
    for line in r.stdout.split('\n'):
        l = line.strip().strip('"')
        if 'Pass:' in l or 'Fail:' in l:
            print(l)
    
    errors = [l for l in r.stderr.split('\n') if 'ERROR' in l]
    print(f"\n{len(errors)} runtime errors")
    
    # Group by what's missing
    missing = {}
    for e in errors:
        m = re.search(r'undefined:\s*(\S+)', e)
        if m:
            k = m.group(1)
            missing[k] = missing.get(k, 0) + 1
        else:
            m = re.search(r'ERROR:\s*(.{30})', e)
            if m:
                k = m.group(1)
                missing[k] = missing.get(k, 0) + 1
    
    if missing:
        print("\nMissing/broken:")
        for k, c in sorted(missing.items(), key=lambda x: -x[1])[:25]:
            print(f"  {k}: {c}")

if __name__ == '__main__':
    main()
