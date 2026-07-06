#!/usr/bin/env python3
pass

L = []
bal = 0

def add(line):
    global bal
    o = line.count('('); c = line.count(')')
    bal += o - c
    L.append(line)

# ── Helpers ──
add("(define (default0 v) (if (= v 0) 0 v))")
for name, key in [("pool-s","p0s"),("pool-a","p0a"),("pool-bs","p0bs"),("pool-ba","p0ba"),("pool-bi","p0bi"),("pool-si","p0si"),("pool-lb","p0lb"),("pool-res","p0res")]:
    add('(define (%s) (near/load "%s"))' % (name, key))
for name, key in [("cfg-vol","c0vol"),("cfg-cf","c0cf"),("cfg-tu","c0tu"),("cfg-r0","c0r0"),("cfg-r1","c0r1"),("cfg-r2","c0r2"),("cfg-rc","c0rc")]:
    add('(define (%s) (default0 (near/load "%s")))' % (name, key))

# ── Health ──
add("(define (health)")
bal += 1
add('  (let ((c (near/kv-get "u/" (near/signer_account_id) "/c")))')
add('  (let ((bs (near/kv-get "u/" (near/signer_account_id) "/b")))')
add("  (let ((dc (default0 c)))")
add("  (let ((dbs (default0 bs)))")
add("  (if (= dbs 0) 10000")
add("  (let ((bi (default0 (pool-bi))))")
add("  (let ((cf (cfg-cf)))")
add("  (let ((vol (cfg-vol)))")
add("  (let ((debt (/ (* dbs bi) 1000000)))")
add("  (if (= debt 0) 10000 (/ (* (* dc cf) vol) (* debt 100)))))))))))))")
bal -= 1

# ── do-accrue ──
add("(define (do-accrue)")
bal += 1
add("  (let ((ba (default0 (pool-ba))))")
add("  (let ((sa (default0 (pool-a))))")
add("  (if (or (= sa 0) (= ba 0)) 0")
add("  (let ((util (/ (* ba 100) sa)))")
add("  (let ((tu (cfg-tu)))")
add("  (let ((r0 (cfg-r0)))")
add("  (let ((r1 (cfg-r1)))")
add("  (let ((r2 (cfg-r2)))")
add('  (let ((rate (if (< util tu) (+ r0 (/ (* (- r1 r0) util) tu)) (+ r1 (/ (* (- r2 r1) (- util tu)) (- 100 tu))))))')
add("  (let ((rc (cfg-rc)))")
add("  (let ((bh (near/block_index)))")
add("  (let ((lb (default0 (pool-lb))))")
add("  (let ((delta (- bh lb)))")
add("  (let ((cd (if (> delta 100) 100 delta)))")
add("  (let ((rd (/ (* rate cd) 10000)))")
add("  (let ((intr (* ba rd)))")
add("  (let ((res (/ (* intr rc) 100)))")
add("  (let ((sintr (- intr res)))")
add("  (let ((bi (default0 (pool-bi))))")
add("  (let ((si (default0 (pool-si))))")
add("  (let ((si_inc (/ (* si sintr) (if (= sa 0) 1 sa))))")
add("  (begin")
add('  (near/store "p0ba" (+ ba intr))')
add('  (near/store "p0a" (+ sa sintr))')
add('  (near/store "p0res" (+ (default0 (pool-res)) res))')
add("  (near/store \"p0bi\" (+ bi (* bi rd)))")
add("  (near/store \"p0si\" (+ si si_inc))")
add("  (near/store \"p0lb\" (+ lb cd))")
add('  (near/log "accrue" intr)')
closes = chr(41) * 23  # 21 lets + begin + define = 23
add("  0" + closes)
bal -= 1

# ── register-asset ──
add("(define (register-asset)")
bal += 1
add('  (let ((locked (default0 (near/load "c0locked"))))')
add('  (if (= locked 1) (near/return_str "already_configured")')
add("  (let ((inp (near/input)))")
add("  (let ((vol (bytes-to-u32 (str-slice inp 0 4))))")
add("  (let ((cf (bytes-to-u32 (str-slice inp 4 8))))")
add("  (let ((tu (bytes-to-u32 (str-slice inp 8 12))))")
add("  (let ((r0 (bytes-to-u32 (str-slice inp 12 16))))")
add("  (let ((r1 (bytes-to-u32 (str-slice inp 16 20))))")
add("  (let ((r2 (bytes-to-u32 (str-slice inp 20 24))))")
add("  (let ((rc (bytes-to-u32 (str-slice inp 24 28))))")
add("  (if (or (> cf 100) (> tu 100) (> vol 100000) (> r2 5000) (> rc 50))")
add('  (near/return_str "invalid_config")')
add("  (begin")
add('  (near/store "c0vol" vol)')
add('  (near/store "c0cf" cf)')
add('  (near/store "c0tu" tu)')
add('  (near/store "c0r0" r0)')
add('  (near/store "c0r1" r1)')
add('  (near/store "c0r2" r2)')
add('  (near/store "c0rc" rc)')
add('  (near/store "c0locked" 1)')
add('  (near/log "register_asset" vol)')
add('  (near/return_str "ok")))))))))))))))))')
bal -= 1

# ── init ──
add("(define (init)")
bal += 1
add("  (begin")
add('  (near/store "p0s" 0)')
add('  (near/store "p0a" 0)')
add('  (near/store "p0bs" 0)')
add('  (near/store "p0ba" 0)')
add('  (near/store "p0bi" 1000000)')
add('  (near/store "p0si" 1000000)')
add("  (near/store \"p0lb\" (near/block_index))")
add('  (near/store "p0res" 0)')
add('  (near/store "c0locked" 0)')
add('  (near/kv "u/" (near/signer_account_id) "/s" 0)')
add('  (near/kv "u/" (near/signer_account_id) "/c" 0)')
add('  (near/kv "u/" (near/signer_account_id) "/b" 0)')
add('  (near/log "init" (near/block_index))')
add('  (near/return_str "ok"))))')
bal -= 1

# ── deposit ──
add("(define (deposit)")
bal += 1
add("  (if (near/deposit-gte 1)")
add("  (let ((inp (near/input)))")
add("  (let ((amt (bytes-to-u32 (str-slice inp 0 4))))")
add("  (let ((ps (default0 (pool-s))))")
add("  (let ((pa (default0 (pool-a))))")
add("  (let ((si (default0 (pool-si))))")
add("  (let ((sh (/ (* amt 1000000) (if (= si 0) 1 si))))")
add('  (let ((cs (near/kv-get "u/" (near/signer_account_id) "/s")))')
add("  (begin")
add("  (do-accrue)")
add('  (near/store "p0s" (+ ps sh))')
add('  (near/store "p0a" (+ pa amt))')
add('  (near/kv "u/" (near/signer_account_id) "/s" (+ (default0 cs) sh))')
add('  (near/log "deposit" amt)')
add('  (near/return_str (to-string sh)))))))))))))')
add('  (near/return_str "deposit_no_funds")))')
bal -= 1

# ── withdraw ──
add("(define (withdraw)")
bal += 1
add("  (do-accrue)")
add("  (let ((inp (near/input)))")
add("  (let ((ws (bytes-to-u32 (str-slice inp 0 4))))")
add('  (let ((cs (near/kv-get "u/" (near/signer_account_id) "/s")))')
add("  (let ((cur (default0 cs)))")
add('  (if (> ws cur) (near/return_str "too_much")')
add("  (let ((si (default0 (pool-si))))")
add("  (let ((wa (/ (* ws si) 1000000)))")
add("  (let ((pa (default0 (pool-a))))")
add("  (let ((maxw (/ (* pa (- 100 (cfg-tu))) 100)))")
add('  (if (> wa maxw) (near/return_str "min_liquidity")')
add("  (let ((ps (default0 (pool-s))))")
add("  (begin")
add('  (near/kv "u/" (near/signer_account_id) "/s" (- cur ws))')
add('  (near/store "p0s" (- ps ws))')
add('  (near/store "p0a" (- pa wa))')
add("  (near/write_amount wa)")
add("  (let ((slen (near/signer_to_buf)))")
add("  (let ((pidx (near/promise_batch_create 4096 slen)))")
add("  (near/promise_batch_action_transfer pidx 256 16)")
add('  (near/log "withdraw" wa)')
add('  (near/return_str (to-string wa)))))))))))))))))')
bal -= 1

# ── inc-collat ──
add("(define (inc-collat)")
bal += 1
add("  (begin")
add("  (do-accrue)")
add("  (let ((inp (near/input)))")
add("  (let ((amt (bytes-to-u32 (str-slice inp 0 4))))")
add('  (let ((cc (near/kv-get "u/" (near/signer_account_id) "/c")))')
add('  (near/kv "u/" (near/signer_account_id) "/c" (+ (default0 cc) amt))')
add('  (near/log "inc_collat" amt)')
add('  (near/return_str "ok"))))))))')
bal -= 1

# ── dec-collat ──
add("(define (dec-collat)")
bal += 1
add("  (do-accrue)")
add("  (let ((inp (near/input)))")
add("  (let ((amt (bytes-to-u32 (str-slice inp 0 4))))")
add('  (let ((cc (near/kv-get "u/" (near/signer_account_id) "/c")))')
add("  (let ((cur (default0 cc)))")
add('  (if (> amt cur) (near/return_str "too_much")')
add("  (let ((nc (- cur amt)))")
add('  (near/kv "u/" (near/signer_account_id) "/c" nc)')
add("  (let ((h (health)))")
add("  (if (< h 100)")
add("  (begin")
add('  (near/kv "u/" (near/signer_account_id) "/c" cur)')
add('  (near/return_str "unsafe"))')
add("  (begin")
add('  (near/log "dec_collat" amt)')
add('  (near/return_str "ok")))))))))))))')
bal -= 1

# ── borrow ──
add("(define (borrow)")
bal += 1
add("  (do-accrue)")
add("  (let ((inp (near/input)))")
add("  (let ((amt (bytes-to-u32 (str-slice inp 0 4))))")
add("  (let ((bs (default0 (pool-bs))))")
add("  (let ((ba (default0 (pool-ba))))")
add('  (let ((ub (near/kv-get "u/" (near/signer_account_id) "/b")))')
add("  (let ((bi (default0 (pool-bi))))")
add("  (let ((cub (default0 ub)))")
add("  (let ((sh (/ (* amt 1000000) (if (= bi 0) 1 bi))))")
add('  (near/kv "u/" (near/signer_account_id) "/b" (+ cub sh))')
add('  (near/store "p0ba" (+ ba amt))')
add("  (let ((h (health)))")
add("  (if (< h 100)")
add("  (begin")
add('  (near/kv "u/" (near/signer_account_id) "/b" cub)')
add('  (near/store "p0ba" ba)')
add('  (near/return_str "unsafe"))')
add("  (begin")
add('  (near/store "p0bs" (+ bs sh))')
add('  (near/log "borrow" amt)')
add('  (near/return_str (to-string sh))))))))))))))')
bal -= 1

# ── repay ──
add("(define (repay)")
bal += 1
add("  (do-accrue)")
add("  (let ((inp (near/input)))")
add("  (let ((amt (bytes-to-u32 (str-slice inp 0 4))))")
add("  (let ((bs (default0 (pool-bs))))")
add("  (let ((ba (default0 (pool-ba))))")
add("  (let ((bi (default0 (pool-bi))))")
add('  (if (= ba 0) (near/return_str "no_debt")')
add('  (if (> amt ba) (near/return_str "too_much")')
add("  (let ((sh (/ (* amt 1000000) (if (= bi 0) 1 bi))))")
add('  (let ((cb (near/kv-get "u/" (near/signer_account_id) "/b")))')
add("  (let ((cub (default0 cb)))")
add('  (if (> sh cub) (near/return_str "too_much")')
add("  (begin")
add('  (near/kv "u/" (near/signer_account_id) "/b" (- cub sh))')
add('  (near/store "p0bs" (- bs sh))')
add('  (near/store "p0ba" (- ba amt))')
add('  (near/log "repay" amt)')
add('  (near/return_str (to-string sh))))))))))))))')
bal -= 1

# ── liquidate ──
add("(define (liquidate)")
bal += 1
add("  (do-accrue)")
add("  (let ((inp (near/input)))")
add("  (let ((tlen (bytes-to-u32 (str-slice inp 0 4))))")
add("  (let ((target (str-slice inp 4 (+ 4 tlen))))")
add("  (let ((ra (bytes-to-u32 (str-slice inp (+ 4 tlen) (+ 8 tlen)))))")
add("  (let ((ms (bytes-to-u32 (str-slice inp (+ 8 tlen) (+ 12 tlen)))))")
add("  (let ((h (health)))")
add('  (if (>= h 100) (near/return_str "healthy")')
add('  (let ((vc (near/kv-get "u/" target "/c")))')
add('  (let ((vb (near/kv-get "u/" target "/b")))')
add("  (let ((cvc (default0 vc)))")
add("  (let ((cvb (default0 vb)))")
add("  (let ((bi (default0 (pool-bi))))")
add("  (let ((vd (/ (* cvb bi) 1000000)))")
add('  (if (= vd 0) (near/return_str "no_debt")')
add("  (let ((ar (if (> ra vd) vd ra)))")
add("  (let ((dc (+ 5 (/ (* (- 100 h) 15) 100))))")
add("  (let ((sa (/ (* ar dc) 100)))")
add("  (let ((a1 (if (> sa ms) ms sa)))")
add("  (let ((a2 (if (> a1 cvc) cvc a1)))")
add("  (let ((bs (default0 (pool-bs))))")
add("  (let ((ba (default0 (pool-ba))))")
add("  (let ((sh (/ (* ar 1000000) (if (= bi 0) 1 bi))))")
add("  (begin")
add('  (near/kv "u/" target "/b" (- cvb sh))')
add('  (near/store "p0bs" (- bs sh))')
add('  (near/store "p0ba" (- ba ar))')
add('  (near/kv "u/" target "/c" (- cvc a2))')
add('  (near/log "liquidate" a2)')
add('  (near/return_str (to-string a2))))))))))))))))))))))))))))))')
bal -= 1

# ── Views ──
add('(define (get-supplied) (let ((us (near/kv-get "u/" (near/signer_account_id) "/s"))) (let ((si (default0 (pool-si)))) (near/return_str (to-string (/ (* (default0 us) (if (= si 0) 1 si)) 1000000))))))')
add('(define (get-collat) (let ((uc (near/kv-get "u/" (near/signer_account_id) "/c"))) (near/return_str (to-string (default0 uc))))))')
add('(define (get-borr) (let ((ub (near/kv-get "u/" (near/signer_account_id) "/b"))) (let ((bi (default0 (pool-bi)))) (near/return_str (to-string (/ (* (default0 ub) (if (= bi 0) 1 bi)) 1000000))))))')
add('(define (get-health) (near/return_str (to-string (health))))')
add('(define (get-pool-s) (near/return_str (to-string (default0 (pool-s)))))')
add('(define (get-pool-a) (near/return_str (to-string (default0 (pool-a)))))')
add('(define (get-reserve) (near/return_str (to-string (default0 (pool-res)))))')
add('(define (get-borrow-index) (near/return_str (to-string (default0 (pool-bi)))))')
add('(define (get-supply-index) (near/return_str (to-string (default0 (pool-si)))))')
add('(define (get-pool-lb) (near/return_str (to-string (default0 (pool-lb)))))')
add('(define (get-supply-shares) (let ((us (near/kv-get "u/" (near/signer_account_id) "/s"))) (near/return_str (to-string (default0 us))))))')
add('(define (get-borrow-shares) (let ((ub (near/kv-get "u/" (near/signer_account_id) "/b"))) (near/return_str (to-string (default0 ub))))))')
add('(define (get-borrow-rate) (let ((ba (default0 (pool-ba)))) (let ((sa (default0 (pool-a)))) (if (= sa 0) 0 (let ((util (/ (* ba 100) sa))) (let ((tu (cfg-tu))) (let ((r0 (cfg-r0))) (let ((r1 (cfg-r1))) (let ((r2 (cfg-r2))) (near/return_str (to-string (if (< util tu) (+ r0 (/ (* (- r1 r0) util) tu)) (+ r1 (/ (* (- r2 r1) (- util tu)) (- 100 tu)))))))))))))))')
add('(define (get-supply-rate) (let ((ba (default0 (pool-ba)))) (let ((sa (default0 (pool-a)))) (if (= sa 0) 0 (let ((util (/ (* ba 100) sa))) (let ((tu (cfg-tu))) (let ((r0 (cfg-r0))) (let ((r1 (cfg-r1))) (let ((r2 (cfg-r2))) (let ((br (if (< util tu) (+ r0 (/ (* (- r1 r0) util) tu)) (+ r1 (/ (* (- r2 r1) (- util tu)) (- 100 tu)))))) (let ((rc (cfg-rc))) (near/return_str (to-string (/ (* br (- 100 rc)) 100))))))))))))))')
add("")

# ── Exports ──
for name in ["register_asset", "init", "deposit", "withdraw", "inc_collat", "dec_collat", "borrow", "repay", "liquidate", "get_supplied", "get_collat", "get_borr", "get_health", "get_pool", "get_pool_a", "get_borrow_rate", "get_supply_rate", "get_reserve", "get_borrow_index", "get_supply_index", "get_pool_lb", "get_supply_shares", "get_borrow_shares"]:
    add('(export "%s" %s)' % (name, name.replace("_", "-")))

content = "\n".join(L)
o = content.count('('); cl = content.count(')')
print(f"Final: {o}/{cl} balanced={o==cl} tracker_bal={bal}")

if o != cl:
    for i, l in enumerate(content.split("\n"), 1):
        lo = l.count('('); lcl = l.count(')')
        if lo != lcl:
            print(f"  L{i}: {lo}/{lcl} diff={lo-lcl}")
else:
    write_file(path="/Users/asil/.openclaw/workspace/lisp-rlm/burrow-mini.lisp", content=content)
    print(f"OK -- {len(content)} bytes, {len(L)} lines")
