#!/usr/bin/env python3
"""Parse verify_fn.lisp, validate every let-binding is (name expr); report first offense."""
import re

src = open('/tmp/verify_fn.lisp').read()

def parse(s):
    toks = re.findall(r'\(|\)|[^\s()]+', s)
    pos = 0
    def rd():
        nonlocal pos
        t = toks[pos]; pos += 1
        if t == '(':
            lst = []
            while toks[pos] != ')':
                if pos >= len(toks):
                    raise SyntaxError("EOF inside form at tok %d" % pos)
                lst.append(rd())
            pos += 1
            return lst
        if re.fullmatch(r'-?\d+', t):
            return int(t)
        return t
    forms = []
    while pos < len(toks):
        forms.append(rd())
    return forms

def check(form, path="root"):
    if not isinstance(form, list) or not form:
        return
    op = form[0]
    if op == 'let':
        if len(form) < 3:
            print("BAD let arity at", path, ":", len(form), repr(form)[:200])
            return
        binds = form[1]
        if not isinstance(binds, list):
            print("BAD bindings (not list) at", path)
            return
        for b in binds:
            if not (isinstance(b, list) and len(b) == 2 and isinstance(b[0], str)):
                print("BAD binding at", path, ":", repr(b)[:120])
                return
        for b in binds:
            check(b[1], path + "/" + b[0])
        check(form[2], path + "/body")
        return
    if op == 'loop':
        for b in form[1]:
            if not (isinstance(b, list) and len(b) == 2):
                print("BAD loop binding at", path, ":", repr(b)[:80])
                return
        for x in form[2:]:
            check(x, path + "/loop")
        return
    if op in ('if',):
        for x in form[1:]:
            check(x, path + "/if")
        return
    for x in form[1:]:
        check(x, path + "/" + str(op))

forms = parse(src)
print("top-level forms:", len(forms))
for f in forms:
    check(f)
print("validation done")
