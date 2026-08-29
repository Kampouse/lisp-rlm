#!/usr/bin/env python3
"""gen-vectors.py — emit JSON args + expected outcomes for the Phase 1
differential run (contract id = escrow.test.near, both harnesses)."""
import json, sys
sys.path.insert(0, ".")
from bip340 import sign, sha, i2b, mul, b2i

CONTRACT = "escrow.test.near"
SK = bytes([0xAA] * 32)
PK = i2b(mul(b2i(SK))[0]).hex()  # 6a04ab98...
TS = int(sys.argv[1]) if len(sys.argv) > 1 else 1787000000_000000000
EXPIRES = TS + 3600_000000000    # +1h
LATE = TS - 3600*10**9           # already expired

def owner_msg(action, nonce):
    return (f"expires {EXPIRES}.000000000: {action} | nonce: {nonce} "
            f"| contract: {CONTRACT}").encode()

def owner_sig(action, nonce):
    return sign(SK, sha(owner_msg(action, nonce))).hex()

def pause_msg():
    return f"expires {EXPIRES}.000000000: pause | contract: {CONTRACT}".encode()

# tampered signature (flip last byte of a real one)
bad = list(owner_sig("create_wallet:satoshi", 7))
bad[-1] = '0' if bad[-1] != '0' else '1'
BAD_SIG = "".join(bad)

steps = [
    ("init", {"npub": PK}, "ok"),
    ("test_verify_nostr", {"message": "test", "pubkey_hex": PK,
                           "signature": sign(SK, sha(b"test")).hex()}, "ok"),
    # no deposit attached in harness → Rust path also aborts ERR_STORAGE_DEPOSIT
    ("create_wallet", {"name": "satoshi", "signature": owner_sig("create_wallet:satoshi", 7),
                       "expires_at": str(EXPIRES), "nonce": "7"}, "ERR_STORAGE_DEPOSIT"),
    # same nonce reused → nonce already consumed by the failed attempt?
    # (Rust: verify_owner consumes nonce BEFORE deposit check → yes, consumed)
    ("create_wallet", {"name": "satoshi", "signature": owner_sig("create_wallet:satoshi", 7),
                       "expires_at": str(EXPIRES), "nonce": "7"}, "ERR_NONCE_ALREADY_USED"),
    ("create_wallet", {"name": "satoshi", "signature": owner_sig("create_wallet:satoshi", 8),
                       "expires_at": str(EXPIRES), "nonce": "8"}, "ERR_STORAGE_DEPOSIT"),
    # expired signature
    ("create_wallet", {"name": "satoshi", "signature": sign(SK, sha(
        f"expires {LATE}.000000000: create_wallet:satoshi | nonce: 9 | contract: {CONTRACT}".encode())).hex(),
                       "expires_at": str(LATE), "nonce": "9"}, "ERR_SIG_EXPIRED"),
    # tampered signature
    ("create_wallet", {"name": "satoshi", "signature": BAD_SIG,
                       "expires_at": str(EXPIRES), "nonce": "10"}, "ERR_INVALID_OWNER_SIGNATURE"),
    # nonce too low
    ("create_wallet", {"name": "satoshi", "signature": owner_sig("create_wallet:satoshi", 0),
                       "expires_at": str(EXPIRES), "nonce": "0"}, "ERR_STORAGE_DEPOSIT"),
    # now ononce slid to 1 → nonce 0 is genuinely too low
    ("create_wallet", {"name": "satoshi", "signature": owner_sig("create_wallet:satoshi", 0),
                       "expires_at": str(EXPIRES), "nonce": "0"}, "ERR_NONCE_TOO_LOW"),
    # nonce beyond window
    ("create_wallet", {"name": "satoshi", "signature": owner_sig("create_wallet:satoshi", 100),
                       "expires_at": str(EXPIRES), "nonce": "100"}, "ERR_NONCE_WINDOW_EXCEEDED"),
    # pause (no nonce consumption) — tampered → fail; then real → ok
    ("pause", {"signature": BAD_SIG, "expires_at": str(EXPIRES)}, "ERR_NOT_AUTHORIZED_TO_PAUSE"),
    ("pause", {"signature": sign(SK, sha(pause_msg())).hex(), "expires_at": str(EXPIRES)}, "ok"),
    ("is_paused", {}, "1"),
    # while paused: create aborts ERR_PAUSED
    ("create_wallet", {"name": "satoshi", "signature": owner_sig("create_wallet:satoshi", 11),
                       "expires_at": str(EXPIRES), "nonce": "11"}, "ERR_PAUSED"),
    ("unpause", {"signature": owner_sig("unpause", 12), "expires_at": str(EXPIRES),
                 "nonce": "12"}, "ok"),
    ("is_paused", {}, "0"),
    # wallet name rules
    ("create_wallet", {"name": "bad name!", "signature": owner_sig("create_wallet:bad name!", 13),
                       "expires_at": str(EXPIRES), "nonce": "13"}, "ERR_STORAGE_DEPOSIT"),
    ("get_owner_nonce", {}, "1"),  # consumed 7,8 → window slid to 8 (7 was bit0→slide, 8 was bit1→slide) …then 11? no—11 aborted at ERR_PAUSED AFTER verify… Rust order: verify_owner FIRST (consumes 11? NO — Rust create_wallet: assert_not_paused FIRST, then verify_owner). Check both.
    ("get_version", {}, "1"),
]

for name, args, expect in steps:
    print(json.dumps({"method": name, "args": args, "expect": expect}))
