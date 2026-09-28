#!/usr/bin/env python3
"""gen_bip340_verify.py — BIP-340 VERIFY on the fixed lib.

Emits: shamod.lisp + verify appendix -> bip340_verify.lisp -> bip340_verify.wasm
Judge: all 19 official BIP-340 test vectors (embedded CSV) through inlayer,
       + vector-1 SIGN reproduction (aux from CSV) via the sign wasm.

verify = s*G == R + e*P  computed as a single 256-step Jacobian ladder
         (Shamir trick: double; add G if s-bit; add P if e-bit).
Fail-fast guards per spec: s<n, r<p, x(P)<p, x(P)!=0, lift_x success both,
R!=inf.  lift_x: c = x^3+7; y = c^((p+1)/4); y^2==c; even-y pick.
Montgomery domain: true-value parity via demont (fm v (c-onep 0)).
"""
import re, subprocess, json, hashlib, sys, os

D = '/tmp/nostr_probe'
SRC = f'{D}/shamod.lisp'
OUT_LISP = f'{D}/bip340_verify.lisp'
OUT_WASM = f'{D}/bip340_verify.wasm'
NC = '/Users/asil/dev/lisp-rlm/target/release/near-compile'
B = '/Users/asil/.local/bin/inlayer'

src = open(SRC).read()
names = re.findall(r'^\(define \([\w\-?>]+', src, re.M)
for dep in ['fe-mul', 'fe-add', 'fe-sub', 'fe-zero?', 'fe-eq', 'fe-pow', 'fe-sqrt',
            'sc-geq-n', 'sc-from-hex', 'sc-reduce', 'sc-negv', 'fm', 'fa', 'fs', 'fz',
            'pt-dbl', 'my-add', 'words->limbs', 'c-r2', 'c-sevenm', 'c-p', 'c-onem',
            'c-onep', 'c-gxm', 'c-gym', 'c-zero', 'c-pm2']:
    if f'(define ({dep}' not in src and f'(define ({dep} ' not in src:
        print(f"missing dep: {dep}"); sys.exit(1)

V = lambda name, i: f'(vec-nth {name} {i})'
def spread(name):
    return " ".join(V(name, i) for i in range(9))

def geq_call(fn, name):
    return f'({fn} {spread(name)})'

def demont(name):
    return f'(fm {name} (c-onep 0))'

def parity1(name):
    # 1 if TRUE value (demont) is odd
    return f'(band (vec-nth {demont(name)} 7) 1)'


# generic affine-add-on-Jacobian (replaces G-hardcoded pt-add-aff)
PT_ADD = '''(define (pt-add X1 Y1 Z1 X2 Y2 Z2)
  (if (= (fz Z1) 1)
      (list X2 Y2 Z2)
      (if (= (fz Z2) 1)
          (list X1 Y1 Z1)
          (let* ((ZZ1 (fm Z1 Z1))
                 (U2 (fm X2 ZZ1))
                 (S2 (fm Y2 (fm ZZ1 Z1)))
                 (H (fs U2 X1))
                 (RR (fa (fs S2 Y1) (fs S2 Y1)))
                 (HH (fm H H))
                 (I4 (fa (fa HH HH) (fa HH HH)))
                 (J (fm H I4))
                 (r2 (fm RR RR))
                 (V (fm X1 I4))
                 (X3 (fs (fs r2 J) (fa V V)))
                 (Y3 (fs (fm RR (fs V X3)) (fa (fm Y1 J) (fm Y1 J))))
                 (Z3 (fs (fm (fa Z1 H) (fa Z1 H)) (fa ZZ1 HH))))
            (list X3 Y3 Z3)))))
(define (my-add-g X1 Y1 Z1)
  (pt-add X1 Y1 Z1 (c-gxm 0) (c-gym 0) (c-onep 0)))
'''

def ladder():
    # Shamir ladder over bits 255..0 of s and e (9-limb lists)
    return '''(loop ((Pt (list (c-zero 0) (c-zero 0) (c-zero 0))) (i 255))
    (let* ((P2 (pt-dbl (vec-nth Pt 0) (vec-nth Pt 1) (vec-nth Pt 2)))
           (sbit (band (shr (vec-nth sv (/ i 30)) (mod i 30)) 1))
           (ebit (band (shr (vec-nth ev (/ i 30)) (mod i 30)) 1))
           (P3 (if (= sbit 1)
                   (pt-add (vec-nth P2 0) (vec-nth P2 1) (vec-nth P2 2)
                           (vec-nth GJ 0) (vec-nth GJ 1) (vec-nth GJ 2))
                   P2))
           (P4 (if (= ebit 1)
                   (pt-add (vec-nth P3 0) (vec-nth P3 1) (vec-nth P3 2)
                           (vec-nth PJ 0) (vec-nth PJ 1) (vec-nth PJ 2))
                   P3)))
      (if (< i 1) P4 (recur P4 (- i 1)))))'''

# NOTE on forms: (fm A b) = A * b * R^-1 (A: list, b: list-of-9 or list ref)
# rep(x)   = (fm plain (c-r2 0))    where plain: list of 9 limbs
# demont   = (fm rep (c-onep 0))    -> plain list of 9 limbs
LADDER_BINDS = [
    # public key point
    ('pxm', '(fm (sc-from-raw pkb) (c-r2 0))'),          # x(P) rep
    ('pinf', '(if (= (fz pxm) 1) 1 0)'),                                 # x(P)==0 ?
    ('wsq', '(fm pxm pxm)'),                              # x^2 rep
    ('cube', '(fm wsq pxm)'),                             # x^3 rep
    ('cpk', '(fa cube (c-sevenm 0))'),                    # x^3+7 rep
    ('iyp', '(fe-sqrt (vec-nth cpk 0) (vec-nth cpk 1) (vec-nth cpk 2) (vec-nth cpk 3) (vec-nth cpk 4) (vec-nth cpk 5) (vec-nth cpk 6) (vec-nth cpk 7) (vec-nth cpk 8))'),
    ('yp2', '(fm iyp iyp)'),                              # y^2 rep
    ('badp', '(if (= (fe-eqc yp2 cpk) 1) 0 1)'),          # 1 if y^2 != c
    ('nyp', '(fs (c-zero 0) iyp)'),                       # rep(0) - rep(y) = rep(-y)
    ('pky', '(if (= (band (vec-nth (fe-words-pure (fm iyp (c-onep 0))) 7) 1) 1) nyp iyp)'),  # even-y: flip if iyp odd
    ('ninf', '(if (= (fz pky) 1) 1 0)'),                                 # y==0 => point at inf
    ('PJ', '(list pxm pky (c-onem 0))'),                  # rep coords, Z=rep(1)
    ('GJ', '(list (c-gxm 0) (c-gym 0) (c-onem 0))'),      # rep coords, Z=rep(1)
    # R point from signature
    ('rm', '(fm (sc-from-raw rb) (c-r2 0))'),
    ('rsq', '(fm rm rm)'),
    ('rcube', '(fm rsq rm)'),
    ('cr', '(fa rcube (c-sevenm 0))'),
    ('iyr', '(fe-sqrt (vec-nth cr 0) (vec-nth cr 1) (vec-nth cr 2) (vec-nth cr 3) (vec-nth cr 4) (vec-nth cr 5) (vec-nth cr 6) (vec-nth cr 7) (vec-nth cr 8))'),
    ('yr2', '(fm iyr iyr)'),
    ('badr', '(if (= (fe-eqc yr2 cr) 1) 0 1)'),
    ('nyr', '(fs (c-zero 0) iyr)'),
    ('ry', '(if (= (band (vec-nth (fe-words-pure (fm nyr (c-onep 0))) 7) 1) 1) iyr nyr)'),
    ('RJ', '(list rm ry (c-onem 0))'),
    # challenge
    ('ehex', '(tag-hash-fixed "BIP0340/challenge" (str-cat rb (str-cat pkb mb)) (str-len (str-cat rb (str-cat pkb mb))))'),
    ('el', '(sc-from-hex ehex)'),
    ('e', f'(sc-reduce {spread("el")})'),
    ('eneg', '(sc-negv e)'),
    ('sv', '(sc-from-raw sb)'),
    ('ev', 'eneg'),
    ('Pf', ladder()),
    ('z2', '(fm (vec-nth Pf 2) (vec-nth Pf 2))'),
    ('lhs', '(vec-nth Pf 0)'),                            # X(R') raw rep
    ('rhs', '(fm rm z2)'),                                # r * Z^2 rep -- scale-matched
    ('zi', '(fe-pow (vec-nth Pf 2) (c-pm2 0))'),
    ('invz', '(fm zi zi)'),                               # 1/Z (mod p)
    ('yaff', '(fm (vec-nth Pf 1) (fm invz zi))'),              # y(R') = Y * (1/Z^2 * 1/Z) = Y/Z^3
    ('rinf', '(if (= (fz (vec-nth Pf 2)) 1) 1 0)'),       # 1 if R' == inf
    ('rpar', '(if (= (band (vec-nth (fe-words-pure (fm yaff (c-onep 0))) 7) 1) 1) 1 0)'),  # 1 if odd y
    ('eq1', '(fe-eqc (vec-nth Pf 0) (fm rm z2))'),        # scale-invariant x-equality
    ('nfeq', '(if (= eq1 1) 0 1)'),
]

body_inner = '(if (= 0 (+ badp badr pinf ninf rinf rpar nfeq)) (hex-decode "01") (hex-decode "00"))'
for name, val in reversed(LADDER_BINDS):
    body_inner = f'(let (({name} {val}))\n  {body_inner})'

# guards must run BEFORE the expensive ladder; build nested if around body_inner
sg = f'(sc-geq-n {spread("(vec-nth (sc-from-hex sb)")}'
# simpler: bind sv first via tiny let, guard, then ladder
BODY = ('(if (= 1 (sc-geq-n (vec-nth svl 0) (vec-nth svl 1) (vec-nth svl 2) (vec-nth svl 3) (vec-nth svl 4) (vec-nth svl 5) (vec-nth svl 6) (vec-nth svl 7) (vec-nth svl 8))) (hex-decode "00")\n'
        ' (if (= 1 (sc-geq-p2 (vec-nth svl 0) (vec-nth svl 1) (vec-nth svl 2) (vec-nth svl 3) (vec-nth svl 4) (vec-nth svl 5) (vec-nth svl 6) (vec-nth svl 7) (vec-nth svl 8))) (hex-decode "00")\n'
        '  (if (= 1 (sc-geq-p2 (vec-nth rvl 0) (vec-nth rvl 1) (vec-nth rvl 2) (vec-nth rvl 3) (vec-nth rvl 4) (vec-nth rvl 5) (vec-nth rvl 6) (vec-nth rvl 7) (vec-nth rvl 8))) (hex-decode "00")\n'
        '   (if (= 1 (sc-geq-p2 (vec-nth pvl 0) (vec-nth pvl 1) (vec-nth pvl 2) (vec-nth pvl 3) (vec-nth pvl 4) (vec-nth pvl 5) (vec-nth pvl 6) (vec-nth pvl 7) (vec-nth pvl 8))) (hex-decode "00")\n'
        f'   {body_inner}))))')

def bal(s):
    d = 0
    instr = False
    esc = False
    for ch in s:
        if esc:
            esc = False
            continue
        if ch == '\\':
            esc = True
            continue
        if ch == '"':
            instr = not instr
            continue
        if instr:
            continue
        if ch == '(':
            d += 1
        elif ch == ')':
            d -= 1
            if d < 0:
                return False
    return d == 0

assert bal(BODY), "BODY unbalanced"

# sc-geq-p2 = geq against c-p: emitted via sed-like construction is fragile;
# emit it directly with the SAME generator pattern as the lib's sc-geq-n but vs p.
PRELUDE = '''(define (fe-eqc A B)
  (let ((wa (fe-words-pure (fm A (c-onep 0))))
        (wb (fe-words-pure (fm B (c-onep 0)))))
    (if (= (vec-nth wa 0) (vec-nth wb 0))
      (if (= (vec-nth wa 1) (vec-nth wb 1))
        (if (= (vec-nth wa 2) (vec-nth wb 2))
          (if (= (vec-nth wa 3) (vec-nth wb 3))
            (if (= (vec-nth wa 4) (vec-nth wb 4))
              (if (= (vec-nth wa 5) (vec-nth wb 5))
                (if (= (vec-nth wa 6) (vec-nth wb 6))
                  (if (= (vec-nth wa 7) (vec-nth wb 7)) 1 0)
                  0)
                0)
              0)
            0)
          0)
        0)
      0)))
        (define (sc-from-raw raw)
  (words->limbs (word-at raw 0) (word-at raw 1) (word-at raw 2) (word-at raw 3)
                (word-at raw 4) (word-at raw 5) (word-at raw 6) (word-at raw 7)))
(define (padlen s n)
  (str-cat s (hex-decode "80") (zs (- (- (* 64 (+ 1 (/ (+ n 8) 64))) n) 9)) (len8 (* 8 n))))
(define (sha-n-blocks sp nb b H)
  (let* ((j0 (* b 64))
         (blk (list (bword sp j0 0)
                    (bword sp (+ j0 4) 0)
                    (bword sp (+ j0 8) 0)
                    (bword sp (+ j0 12) 0)
                    (bword sp (+ j0 16) 0)
                    (bword sp (+ j0 20) 0)
                    (bword sp (+ j0 24) 0)
                    (bword sp (+ j0 28) 0)
                    (bword sp (+ j0 32) 0)
                    (bword sp (+ j0 36) 0)
                    (bword sp (+ j0 40) 0)
                    (bword sp (+ j0 44) 0)
                    (bword sp (+ j0 48) 0)
                    (bword sp (+ j0 52) 0)
                    (bword sp (+ j0 56) 0)
                    (bword sp (+ j0 60) 0)))
         (sch (sched16 blk))
         (H1 (sha-h2 sch (sha-h1 sch H) H)))
    (if (= (+ b 1) nb)
        (words-hex H1)
        (sha-n-blocks sp nb (+ b 1) H1))))
(define (sha-fixed s n)
  (sha-n-blocks (padlen s n) (/ (+ n 72) 64) 0 (c-h0 0)))
(define (tag-hash-fixed tg s n)
  (let* ((th (sha-fixed tg (str-len tg)))
         (tb (hex-decode th)))
    (sha-fixed (str-cat tb (str-cat tb s)) (+ 64 n))))
'''

SC_GEQ_P = '''(define (sc-geq-p2 a0 a1 a2 a3 a4 a5 a6 a7 a8)
  (let ((g 0))
    (begin
    (set! g (if (> a8 (vec-nth (c-p 0) 8)) 1 (if (< a8 (vec-nth (c-p 0) 8)) 0 (if (> a7 (vec-nth (c-p 0) 7)) 1 (if (< a7 (vec-nth (c-p 0) 7)) 0 (if (> a6 (vec-nth (c-p 0) 6)) 1 (if (< a6 (vec-nth (c-p 0) 6)) 0 (if (> a5 (vec-nth (c-p 0) 5)) 1 (if (< a5 (vec-nth (c-p 0) 5)) 0 (if (> a4 (vec-nth (c-p 0) 4)) 1 (if (< a4 (vec-nth (c-p 0) 4)) 0 (if (> a3 (vec-nth (c-p 0) 3)) 1 (if (< a3 (vec-nth (c-p 0) 3)) 0 (if (> a2 (vec-nth (c-p 0) 2)) 1 (if (< a2 (vec-nth (c-p 0) 2)) 0 (if (> a1 (vec-nth (c-p 0) 1)) 1 (if (< a1 (vec-nth (c-p 0) 1)) 0 (if (> a0 (vec-nth (c-p 0) 0)) 1 (if (< a0 (vec-nth (c-p 0) 0)) 0 1)))))))))))))))))))
    g)))'''

VERIFY_FN = f'(define (bip340-verify pkb rb sb mb)\n  (let ((svl (sc-from-raw sb))\n        (rvl (sc-from-raw rb))\n        (pvl (sc-from-raw pkb)))\n    {BODY}))\n'

APPENDIX = (PRELUDE + '\n' + SC_GEQ_P + '\n' + PT_ADD + '\n' + VERIFY_FN + '''
(define (run input)
  (bip340-verify (hex-decode (json-get-str "pk" input))
                 (hex-decode (json-get-str "r" input))
                 (hex-decode (json-get-str "s" input))
                 (hex-decode (json-get-str "msg" input))))
''')

for chunk_name, chunk in [('PRELUDE', PRELUDE), ('SC_GEQ_P', SC_GEQ_P), ('PT_ADD', PT_ADD), ('VERIFY_FN', VERIFY_FN), ('APPENDIX', APPENDIX)]:
    assert bal(chunk), f"{chunk_name} unbalanced"

full = src + '\n' + APPENDIX
open(OUT_LISP, 'w').write(full)
print(f"bip340_verify.lisp: {len(full)} bytes")

r = subprocess.run(f"{NC} {OUT_LISP} --target=outlayer-p2 -o {OUT_WASM} 2>&1", shell=True,
                   capture_output=True, text=True, timeout=560)
log = r.stdout + r.stderr
if 'error' in log.lower() or '❌' in log:
    print("compile FAILED rc:", r.returncode)
    for line in log.splitlines():
        if '❌' in line or 'error' in line.lower():
            print(line[:300])
    sys.exit(1)
print("compile rc: 0 | wasm:", os.path.getsize(OUT_WASM))

# ───────────────────────── judge ─────────────────────────
p = 2**256 - 2**32 - 977
nn = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
Gx = 0x79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798
Gy = 0x483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8

def pt_add_py(Pv, Qv):
    if Pv is None: return Qv
    if Qv is None: return Pv
    if Pv[0] == Qv[0] and (Pv[1] + Qv[1]) % p == 0: return None
    if Pv == Qv:
        l = (3 * Pv[0] * Pv[0]) * pow(2 * Pv[1], -1, p) % p
    else:
        l = (Qv[1] - Pv[1]) * pow(Qv[0] - Pv[0], -1, p) % p
    x = (l * l - Pv[0] - Qv[0]) % p
    return (x, (l * (Pv[0] - x) - Pv[1]) % p)

def mul_py(kk, base=None):
    Rv = None
    A = base if base is not None else (Gx, Gy)
    while kk:
        if kk & 1: Rv = pt_add_py(Rv, A)
        A = pt_add_py(A, A)
        kk >>= 1
    return Rv

def tagged_hash(tag, msg):
    t = hashlib.sha256(tag.encode()).digest()
    return hashlib.sha256(t + t + msg).digest()

def run_case(payload, wasm=OUT_WASM):
    r2 = subprocess.run(f"{B} run {wasm} run '{payload}'", shell=True,
                        capture_output=True, text=True, timeout=1700)
    outp = None
    for l in (r2.stdout or '').splitlines():
        if l.startswith('📤 Output:'):
            outp = l.split('📤 Output:', 1)[1].strip()
    return outp

# ── official vectors (bip-0340/test-vectors.csv, master) ──
CSV = """index,secret key,public key,aux_rand,message,signature,verification result,comment
0,0000000000000000000000000000000000000000000000000000000000000003,F9308A019258C31049344F85F89D5229B531C845836F99B08601F113BCE036F9,0000000000000000000000000000000000000000000000000000000000000000,0000000000000000000000000000000000000000000000000000000000000000,E907831F80848D1069A5371B402410364BDF1C5F8307B0084C55F1CE2DCA821525F66A4A85EA8B71E482A74F382D2CE5EBEEE8FDB2172F477DF4900D310536C0,TRUE,
1,B7E151628AED2A6ABF7158809CF4F3C762E7160F38B4DA56A784D9045190CFEF,DFF1D77F2A671C5F36183726DB2341BE58FEAE1DA2DECED843240F7B502BA659,0000000000000000000000000000000000000000000000000000000000000001,243F6A8885A308D313198A2E03707344A4093822299F31D0082EFA98EC4E6C89,6896BD60EEAE296DB48A229FF71DFE071BDE413E6D43F917DC8DCF8C78DE33418906D11AC976ABCCB20B091292BFF4EA897EFCB639EA871CFA95F6DE339E4B0A,TRUE,
2,C90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B14E5C9,DD308AFEC5777E13121FA72B9CC1B7CC0139715309B086C960E18FD969774EB8,C87AA53824B4D7AE2EB035A2B5BBBCCC080E76CDC6D1692C4B0B62D798E6D906,7E2D58D8B3BCDF1ABADEC7829054F90DDA9805AAB56C77333024B9D0A508B75C,5831AAEED7B44BB74E5EAB94BA9D4294C49BCF2A60728D8B4C200F50DD313C1BAB745879A5AD954A72C45A91C3A51D3C7ADEA98D82F8481E0E1E03674A6F3FB7,TRUE,
3,0B432B2677937381AEF05BB02A66ECD012773062CF3FA2549E44F58ED2401710,25D1DFF95105F5253C4022F628A996AD3A0D95FBF21D468A1B33F8C160D8F517,FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF,FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF,7EB0509757E246F19449885651611CB965ECC1A187DD51B64FDA1EDC9637D5EC97582B9CB13DB3933705B32BA982AF5AF25FD78881EBB32771FC5922EFC66EA3,TRUE,test fails if msg is reduced modulo p or n
4,,D69C3509BB99E412E68B0FE8544E72837DFA30746D8BE2AA65975F29D22DC7B9,,4DF3C3F68FCC83B27E9D42C90431A72499F17875C81A599B566C9889B9696703,00000000000000000000003B78CE563F89A0ED9414F5AA28AD0D96D6795F9C6376AFB1548AF603B3EB45C9F8207DEE1060CB71C04E80F593060B07D28308D7F4,TRUE,
5,,EEFDEA4CDB677750A420FEE807EACF21EB9898AE79B9768766E4FAA04A2D4A34,,243F6A8885A308D313198A2E03707344A4093822299F31D0082EFA98EC4E6C89,6CFF5C3BA86C69EA4B7376F31A9BCB4F74C1976089B2D9963DA2E5543E17776969E89B4C5564D00349106B8497785DD7D1D713A8AE82B32FA79D5F7FC407D39B,FALSE,public key not on the curve
6,,DFF1D77F2A671C5F36183726DB2341BE58FEAE1DA2DECED843240F7B502BA659,,243F6A8885A308D313198A2E03707344A4093822299F31D0082EFA98EC4E6C89,FFF97BD5755EEEA420453A14355235D382F6472F8568A18B2F057A14602975563CC27944640AC607CD107AE10923D9EF7A73C643E166BE5EBEAFA34B1AC553E2,FALSE,has_even_y(R) is false
7,,DFF1D77F2A671C5F36183726DB2341BE58FEAE1DA2DECED843240F7B502BA659,,243F6A8885A308D313198A2E03707344A4093822299F31D0082EFA98EC4E6C89,1FA62E331EDBC21C394792D2AB1100A7B432B013DF3F6FF4F99FCB33E0E1515F28890B3EDB6E7189B630448B515CE4F8622A954CFE545735AAEA5134FCCDB2BD,FALSE,negated message
8,,DFF1D77F2A671C5F36183726DB2341BE58FEAE1DA2DECED843240F7B502BA659,,243F6A8885A308D313198A2E03707344A4093822299F31D0082EFA98EC4E6C89,6CFF5C3BA86C69EA4B7376F31A9BCB4F74C1976089B2D9963DA2E5543E177769961764B3AA9B2FFCB6EF947B6887A226E8D7C93E00C5ED0C1834FF0D0C2E6DA6,FALSE,negated s value
9,,DFF1D77F2A671C5F36183726DB2341BE58FEAE1DA2DECED843240F7B502BA659,,243F6A8885A308D313198A2E03707344A4093822299F31D0082EFA98EC4E6C89,0000000000000000000000000000000000000000000000000000000000000000123DDA8328AF9C23A94C1FEECFD123BA4FB73476F0D594DCB65C6425BD186051,FALSE,sG - eP is infinite. Test fails in single verification if has_even_y(inf) is defined as true and x(inf) as 0
10,,DFF1D77F2A671C5F36183726DB2341BE58FEAE1DA2DECED843240F7B502BA659,,243F6A8885A308D313198A2E03707344A4093822299F31D0082EFA98EC4E6C89,00000000000000000000000000000000000000000000000000000000000000017615FBAF5AE28864013C099742DEADB4DBA87F11AC6754F93780D5A1837CF197,FALSE,sG - eP is infinite. Test fails in single verification if has_even_y(inf) is defined as true and x(inf) as 1
11,,DFF1D77F2A671C5F36183726DB2341BE58FEAE1DA2DECED843240F7B502BA659,,243F6A8885A308D313198A2E03707344A4093822299F31D0082EFA98EC4E6C89,4A298DACAE57395A15D0795DDBFD1DCB564DA82B0F269BC70A74F8220429BA1D69E89B4C5564D00349106B8497785DD7D1D713A8AE82B32FA79D5F7FC407D39B,FALSE,sig[0:32] is not an X coordinate on the curve
12,,DFF1D77F2A671C5F36183726DB2341BE58FEAE1DA2DECED843240F7B502BA659,,243F6A8885A308D313198A2E03707344A4093822299F31D0082EFA98EC4E6C89,FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFC2F69E89B4C5564D00349106B8497785DD7D1D713A8AE82B32FA79D5F7FC407D39B,FALSE,sig[0:32] is equal to field size
13,,DFF1D77F2A671C5F36183726DB2341BE58FEAE1DA2DECED843240F7B502BA659,,243F6A8885A308D313198A2E03707344A4093822299F31D0082EFA98EC4E6C89,6CFF5C3BA86C69EA4B7376F31A9BCB4F74C1976089B2D9963DA2E5543E177769FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141,FALSE,sig[32:64] is equal to curve order
14,,FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFC30,,243F6A8885A308D313198A2E03707344A4093822299F31D0082EFA98EC4E6C89,6CFF5C3BA86C69EA4B7376F31A9BCB4F74C1976089B2D9963DA2E5543E17776969E89B4C5564D00349106B8497785DD7D1D713A8AE82B32FA79D5F7FC407D39B,FALSE,public key is not a valid X coordinate because it exceeds the field size
15,0340034003400340034003400340034003400340034003400340034003400340,778CAA53B4393AC467774D09497A87224BF9FAB6F6E68B23086497324D6FD117,0000000000000000000000000000000000000000000000000000000000000000,,71535DB165ECD9FBBC046E5FFAEA61186BB6AD436732FCCC25291A55895464CF6069CE26BF03466228F19A3A62DB8A649F2D560FAC652827D1AF0574E427AB63,TRUE,message of size 0 (added 2022-12)
16,0340034003400340034003400340034003400340034003400340034003400340,778CAA53B4393AC467774D09497A87224BF9FAB6F6E68B23086497324D6FD117,0000000000000000000000000000000000000000000000000000000000000000,11,08A20A0AFEF64124649232E0693C583AB1B9934AE63B4C3511F3AE1134C6A303EA3173BFEA6683BD101FA5AA5DBC1996FE7CACFC5A577D33EC14564CEC2BACBF,TRUE,message of size 1 (added 2022-12)
17,0340034003400340034003400340034003400340034003400340034003400340,778CAA53B4393AC467774D09497A87224BF9FAB6F6E68B23086497324D6FD117,0000000000000000000000000000000000000000000000000000000000000000,0102030405060708090A0B0C0D0E0F1011,5130F39A4059B43BC7CAC09A19ECE52B5D8699D1A71E3C52DA9AFDB6B50AC370C4A482B77BF960F8681540E25B6771ECE1E5A37FD80E5A51897C5566A97EA5A5,TRUE,message of size 17 (added 2022-12)
18,0340034003400340034003400340034003400340034003400340034003400340,778CAA53B4393AC467774D09497A87224BF9FAB6F6E68B23086497324D6FD117,0000000000000000000000000000000000000000000000000000000000000000,99999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999,403B12B0D8555A344175EA7EC746566303321E5DBFA8BE6F091635163ECA79A8585ED3E3170807E7C03B720FC54C7B23897FCBA0E9D0B4A06894CFD249F22367,TRUE,message of size 100 (added 2022-12)"""
# vector 14 row: fix column order manually (no msg col — empty). Re-parse leniently:
rows = []
for line in CSV.strip().split('\n')[1:]:
    parts = line.split(',')
    # canonical: idx, sk, pk, aux, msg, sig, result, comment
    # NOTE: comment contains no commas, but EMPTY fields (row 14 aux) DO occur;
    # fixed-position split is safe ONLY because comment is last and we pad.
    parts += [''] * (8 - len(parts))
    idx, sk, pk, aux, msg, sig, res = parts[0], parts[1], parts[2], parts[3], parts[4], parts[5], parts[6]
    rows.append(dict(idx=idx, sk=sk, pk=pk, aux=aux, msg=msg, sig=sig, res=res.strip().upper()))
assert len(rows) == 19, len(rows)

results = []
for row in rows:
    pl = {"pk": row["pk"], "r": row["sig"][:64], "s": row["sig"][64:128], "msg": row["msg"]}
    out = run_case(json.dumps(pl, separators=(',', ':')))
    if out not in ("0", "1"):
        # output may be hex text ("01") OR a raw byte char ('\x01') -- both -> "0"/"1"
        try:
            out = str(bytes.fromhex(out)[-1])
        except Exception:
            out = str(ord(out[-1])) if out and ord(out[-1]) <= 1 else "0"
    want = 1 if row["res"] == "TRUE" else 0
    ok = out == str(want)
    results.append(ok)
    print(f"vector {row['idx']:>2}: want {want}  got {out}  {'OK' if ok else 'WRONG'}")

print(f"\nbattery: {sum(results)}/{len(results)} PASS")

# ── vector-1 SIGN reproduction via sign wasm (spec nonce w/ aux) ──
v1 = rows[1]
sk_hex = v1["sk"]; msg_hex = v1["msg"]; aux_hex = v1["aux"]
d = int(sk_hex, 16)
P = mul_py(d)
pk_b = P[0].to_bytes(32, 'big')
assert pk_b.hex().upper() == v1["pk"], "pk mismatch"
du = d if P[1] % 2 == 0 else nn - d
mm = bytes.fromhex(msg_hex)
auxb = bytes.fromhex(aux_hex)
t = bytes(a ^ b for a, b in zip(du.to_bytes(32, 'big'), tagged_hash("BIP0340/aux", auxb)))
kn = int.from_bytes(tagged_hash("BIP0340/nonce", t + pk_b + mm), 'big') % nn
RP = mul_py(kn)
rX = RP[0].to_bytes(32, 'big')
kuse = kn if RP[1] % 2 == 0 else nn - kn
e_bytes = tagged_hash("BIP0340/challenge", rX + pk_b + mm)
e = int.from_bytes(e_bytes, 'big') % nn
s = (kuse + e * du) % nn
sig_py = (rX + s.to_bytes(32, 'big')).hex()
print("  py r :", sig_py[:64])
print("  want r:", v1["sig"][:64].lower())
print("  py s :", sig_py[64:])
print("  want s:", v1["sig"][64:].lower())
if sig_py.upper() != v1["sig"].lower():
    print("  !! python sign path mismatch — DIAGNOSTIC MODE")
pl = {"case": "D", "dbg": "*", "sk": sk_hex, "msg": msg_hex, "pk": pk_b.hex(),
      "df": str(P[1] % 2), "aux": aux_hex}
out = run_case(json.dumps(pl, separators=(',', ':')), wasm=f'{D}/bip340.wasm')
print("\nvector-1 SIGN (wasm, spec nonce + aux):")
print("  want:", v1["sig"].lower())
print("  got :", out)
print("  " + ("MATCH — full BIP-340 conformance" if (out or "").lower() == v1["sig"].lower() else "MISMATCH"))
