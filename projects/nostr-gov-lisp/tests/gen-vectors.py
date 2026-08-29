#!/usr/bin/env python3
"""gen-vectors.py — emit JSON args + expected outcomes for the Phase 1
differential run (contract id = escrow.test.near, both harnesses)."""
import json, sys
sys.path.insert(0, ".")
from bip340 import gov_event, sign, sha, i2b, mul, b2i

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

    # ── Phase 1.5: event auth (kind 37500) ────────────────────────
    # valid event create: auth+nonce pass, deposit gate hits (harness 0 deposit)
    ("create_wallet", dict({"name": "evented"},
                           **gov_event(SK, PK, "create_wallet:evented", 20, EXPIRES, CONTRACT)),
     "ERR_STORAGE_DEPOSIT"),
    # replay same nonce → consumed by the event path too
    ("create_wallet", dict({"name": "evented"},
                           **gov_event(SK, PK, "create_wallet:evented", 20, EXPIRES, CONTRACT)),
     "ERR_NONCE_ALREADY_USED"),
    # wrong kind
    ("create_wallet", dict({"name": "evented"},
                           **gov_event(SK, PK, "create_wallet:evented", 21, EXPIRES, CONTRACT,
                                       kind=4040)),
     "ERR_EVENT_KIND"),
    # action tag mismatch (signed for another name)
    ("create_wallet", dict({"name": "evented"},
                           **gov_event(SK, PK, "create_wallet:other", 22, EXPIRES, CONTRACT)),
     "ERR_EVENT_ACTION"),
    # contract tag mismatch
    ("create_wallet", dict({"name": "evented"},
                           **gov_event(SK, PK, "create_wallet:evented", 23, EXPIRES, CONTRACT,
                                       contract_override="elsewhere.test.near")),
     "ERR_EVENT_CONTRACT"),
    # expired event
    ("create_wallet", dict({"name": "evented"},
                           **gov_event(SK, PK, "create_wallet:evented", 24, LATE, CONTRACT)),
     "ERR_SIG_EXPIRED"),
    # tampered sig (flip last byte of the REAL event sig, override "sig")
    ("create_wallet", dict({"name": "evented"},
                           **{**gov_event(SK, PK, "create_wallet:evented", 25, EXPIRES, CONTRACT),
                              "sig": gov_event(SK, PK, "create_wallet:evented", 25, EXPIRES, CONTRACT)["sig"][:-1]
                                    + ("0" if gov_event(SK, PK, "create_wallet:evented", 25, EXPIRES, CONTRACT)["sig"][-1] != "0" else "1")}),
     "ERR_EVENT_SIG_INVALID"),
    # content with a quote → charset guard
    ("create_wallet", dict({"name": "evented"},
                           **gov_event(SK, PK, "create_wallet:evented", 26, EXPIRES, CONTRACT,
                                       content='say "hi"')),
     "ERR_EVENT_SIG_INVALID"),
    # event pause → ok, event unpause → ok (guardian path, no nonce)
    ("pause", gov_event(SK, PK, "pause", 0, EXPIRES, CONTRACT, content="pause it"),
     "ok"),
    ("is_paused", {}, "1"),
    ("pause", gov_event(SK, PK, "unpause", 0, EXPIRES, CONTRACT, content="pause it"),
     "ERR_EVENT_ACTION"),
    ("unpause", gov_event(SK, PK, "unpause", 27, EXPIRES, CONTRACT),
     "ok"),
    ("is_paused", {}, "0"),
    # legacy path still works after event traffic (fresh nonce 28)
    ("create_wallet", {"name": "satoshi", "signature": owner_sig("create_wallet:satoshi", 28),
                       "expires_at": str(EXPIRES), "nonce": "28"}, "ERR_STORAGE_DEPOSIT"),
    # view after event traffic
    ("get_wallet", {"name": "evented"}, ""),
]

for name, args, expect in steps:
    print(json.dumps({"method": name, "args": args, "expect": expect}))
