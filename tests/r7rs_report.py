#!/usr/bin/env python3
"""
R7RS Conformance Report for lisp-rlm
Analyzes the chibi-scheme R7RS test suite and reports what lisp-rlm needs.
"""

import re, os

def main():
    src = '/tmp/chibi-scheme/tests/r7rs-tests.scm'
    if not os.path.exists(src):
        print("Run: git clone --depth 1 https://github.com/ashinn/chibi-scheme.git /tmp/chibi-scheme")
        return
    
    with open(src) as f:
        content = f.read()
    
    # Count (test ...) forms
    tests = re.findall(r'\(test\b', content)
    print(f"Total R7RS test assertions: {len(tests)}")
    
    # Categorize what's needed
    features = {
        # Already supported
        'define': 0, 'lambda': 0, 'if': 0, 'cond': 0, 'let': 0,
        'let*': 0, 'letrec': 0, 'begin': 0, 'set!': 0,
        '+': 0, '-': 0, '*': 0, '/': 0, '=': 0, '<': 0, '>': 0,
        'and': 0, 'or': 0, 'not': 0,
        'car': 0, 'cdr': 0, 'cons': 0, 'list': 0, 'map': 0, 'apply': 0,
        'quote': 0, 'lambda': 0,
        
        # Missing - easy
        'case': 0, 'when': 0, 'unless': 0, 'do': 0,
        
        # Missing - medium
        'call/cc': 0, 'values': 0, 'let-values': 0,
        'make-vector': 0, 'vector-set!': 0, 'vector-ref': 0,
        'delay': 0, 'force': 0,
        'guard': 0,
        
        # Missing - hard
        'syntax-rules': 0, 'define-syntax': 0,
    }
    
    for feat in features:
        features[feat] = len(re.findall(r'\b' + re.escape(feat) + r'\b', content))
    
    print("\n=== Already Supported ===")
    supported = [(f, c) for f, c in features.items() if c > 0 and f in
                 ['define', 'lambda', 'if', 'cond', 'let', 'begin', 'set!',
                  '+', '-', '*', '/', '=', '<', '>', 'and', 'or', 'not',
                  'car', 'cdr', 'cons', 'list', 'map', 'apply', 'quote']]
    for f, c in sorted(supported, key=lambda x: -x[1]):
        print(f"  {f}: {c} uses")
    
    print("\n=== Missing — Easy Fixes ===")
    easy = [(f, c) for f, c in features.items() if c > 0 and f in
            ['case', 'when', 'unless', 'do', 'let*', 'letrec']]
    for f, c in sorted(easy, key=lambda x: -x[1]):
        print(f"  {f}: {c} uses")
    
    print("\n=== Missing — Medium Effort ===")
    medium = [(f, c) for f, c in features.items() if c > 0 and f in
              ['call/cc', 'values', 'let-values', 'make-vector', 'vector-set!',
               'vector-ref', 'delay', 'force', 'guard']]
    for f, c in sorted(medium, key=lambda x: -x[1]):
        print(f"  {f}: {c} uses")
    
    print("\n=== Missing — Hard / Skip ===")
    hard = [(f, c) for f, c in features.items() if c > 0 and f in
            ['syntax-rules', 'define-syntax']]
    for f, c in sorted(hard, key=lambda x: -x[1]):
        print(f"  {f}: {c} uses")
    
    # Top undefined things that would appear as errors
    print("\n=== Top Missing Builtins (by frequency in test suite) ===")
    builtins_r7 = [
        'length', 'append', 'reverse', 'assv', 'memv', 'member', 'assoc',
        'string-length', 'string-append', 'substring', 'string-contains',
        'string-upcase', 'string-downcase', 'string-copy',
        'string->number', 'number->string',
        'null?', 'boolean?', 'pair?', 'list?',
        'zero?', 'positive?', 'negative?', 'even?', 'odd?',
        'abs', 'min', 'max', 'modulo', 'remainder', 'quotient',
        'char->integer', 'integer->char',
        'exact-integer-sqrt', 'expt', 'square',
        'exact', 'inexact', 'exact->inexact', 'inexact->exact',
        'display', 'write', 'newline', 'read',
        'list-ref', 'list-tail',
        'for-each',
        'vector-length', 'vector->list', 'list->vector',
        'values', 'call-with-values',
        'dynamic-wind',
        'call/cc',
        'make-vector',
        'open-input-file', 'open-output-file',
        'file-exists?', 'delete-file',
        'define-record-type',
        'guard',
    ]
    
    builtin_counts = []
    for b in builtins_r7:
        c = len(re.findall(r'\b' + re.escape(b) + r'\b', content))
        if c > 0:
            builtin_counts.append((b, c))
    
    for b, c in sorted(builtin_counts, key=lambda x: -x[1]):
        # Check if we have an alias
        alias = {
            'length': 'len', 'string-length': 'str-length',
            'string-append': 'str-concat', 'substring': 'str-substring',
            'null?': 'nil?', 'boolean?': 'bool?',
            'string->number': 'to-num', 'number->string': 'to-string',
        }.get(b, None)
        status = f'→ {alias} ✅' if alias else '❌ missing'
        print(f"  {b}: {c} uses {status}")

if __name__ == '__main__':
    main()
