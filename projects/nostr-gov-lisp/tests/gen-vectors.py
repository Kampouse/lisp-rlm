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
    # same nonce replayed: the first attempt TRAPPED, so on-chain semantics
    # (2026-09-02: mock now rolls back storage on trap) → the consume is
    # reverted → same deposit failure again. The old expectation
    # (NONCE_ALREADY_USED) encoded the interpreter's persist-on-trap quirk.
    ("create_wallet", {"name": "satoshi", "signature": owner_sig("create_wallet:satoshi", 7),
                       "expires_at": str(EXPIRES), "nonce": "7"}, "ERR_STORAGE_DEPOSIT"),
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
    # nonce 0, unfunded: dies at the deposit gate. NOTE — under chain
    # semantics (trap = full revert, 2026-09-02) nonce 0 is FRESH here:
    # the old TOO_LOW expectation relied on the window having slid via
    # trap-persisted consumes of 7/8, which reverts now. Funding this
    # vector would CREATE the wallet (breaking the phase-2 satoshi
    # vector) — TOO_LOW needs a genuinely slid window to be reachable.
    ("create_wallet", {"name": "satoshi", "signature": owner_sig("create_wallet:satoshi", 0),
                       "expires_at": str(EXPIRES), "nonce": "0"}, "ERR_STORAGE_DEPOSIT"),
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
    ("get_owner_nonce", {}, "0"),  # rollback semantics (2026-09-02): traps
    # revert their nonce consumes — 7/8 died at deposit (reverted), 9/10 bad
    # sigs, 0 too-low, pause/unpause are event-auth'd → no committed legacy
    # nonce → 0. The old "1" encoded persist-on-trap.
    ("get_version", {}, "1"),

    # ── Phase 1.5: event auth (kind 37500) ────────────────────────
    # valid event create: auth+nonce pass, deposit gate hits (harness 0 deposit)
    ("create_wallet", dict({"name": "evented"},
                           **gov_event(SK, PK, "create_wallet:evented", 20, EXPIRES, CONTRACT)),
     "ERR_STORAGE_DEPOSIT"),
    # replay same nonce → first attempt trapped (deposit), rollback reverts
    # the nonce consume → same failure again (chain semantics, 2026-09-02)
    ("create_wallet", dict({"name": "evented"},
                           **gov_event(SK, PK, "create_wallet:evented", 20, EXPIRES, CONTRACT)),
     "ERR_STORAGE_DEPOSIT"),
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
    # legacy path still works after event traffic (fresh nonce 28) —
    # funded this time (deposit field) so wallet "satoshi" EXISTS for Phase 2
    ("create_wallet", {"name": "satoshi", "signature": owner_sig("create_wallet:satoshi", 28),
                       "expires_at": str(EXPIRES), "nonce": "28"}, "ok",
     500000000000000000000000),
    # committed nonce replay: this create SUCCEEDED (deposit funded) so the
    # nonce consume is durable → replay traps NONCE_ALREADY_USED (real
    # replay protection coverage, replaces the pre-rollback vectors)
    ("create_wallet", {"name": "satoshi", "signature": owner_sig("create_wallet:satoshi", 28),
                       "expires_at": str(EXPIRES), "nonce": "28"},
     "ERR_NONCE_ALREADY_USED",
     500000000000000000000000),
    # view after event traffic
    ("get_wallet", {"name": "evented"}, ""),

    # ── Phase 2: proposals, m-of-n approvals, execute ────────────────
    # wallet "satoshi" exists (created at step with deposit in mock),
    # approvers: fresh keys 0xCC/0xDD, threshold 2-of-2.
]

APR1 = bytes([0xCC] * 32)
APR2 = bytes([0xDD] * 32)
APR1_PK = i2b(mul(b2i(APR1))[0]).hex()
APR2_PK = i2b(mul(b2i(APR2))[0]).hex()

def apr_sig(sk, name, pid, ix):
    m = (f"expires {EXPIRES}.000000000: approve:{name}:{pid}:{ix} "
         f"| contract: {CONTRACT}").encode()
    return sign(sk, sha(m)).hex()

steps += [
    # owner sets a 2-of-2 approver set on wallet "satoshi"
    ("set_approvers", {"name": "satoshi", "pks": f"{APR1_PK},{APR2_PK}", "thr": "2",
                       "signature": owner_sig("set_approvers:satoshi", 30),
                       "expires_at": str(EXPIRES), "nonce": "30"}, "ok"),
    # threshold 3 > 2 approvers → invalid
    ("set_approvers", {"name": "satoshi", "pks": f"{APR1_PK},{APR2_PK}", "thr": "3",
                       "signature": owner_sig("set_approvers:satoshi", 31),
                       "expires_at": str(EXPIRES), "nonce": "31"}, "ERR_THRESHOLD_INVALID"),
    # owner proposes 0.05N to rita.test.near (id 0)
    ("propose", {"name": "satoshi", "pexp": str(EXPIRES), "am": "50000000000000000000000",
                 "rc": "rita.test.near",
                 "signature": owner_sig("propose:satoshi:0", 32),
                 "expires_at": str(EXPIRES), "nonce": "32"}, "ok"),
    ("get_proposal", {"name": "satoshi", "id": "0"}, "active"),
    # execute while only-active → rejected
    ("execute", {"name": "satoshi", "id": "0",
                 "signature": owner_sig("execute:satoshi:0", 33),
                 "expires_at": str(EXPIRES), "nonce": "33"}, "ERR_NOT_APPROVED"),
    # wrong-key approver sig → invalid
    ("approve", {"name": "satoshi", "id": "0", "ix": "0", "pubkey_hex": APR1_PK,
                 "signature": apr_sig(APR2, "satoshi", "0", "0"),
                 "expires_at": str(EXPIRES)}, "ERR_APPROVER_SIG_INVALID"),
    # pk not at index → mismatch
    ("approve", {"name": "satoshi", "id": "0", "ix": "0", "pubkey_hex": APR2_PK,
                 "signature": apr_sig(APR1, "satoshi", "0", "0"),
                 "expires_at": str(EXPIRES)}, "ERR_APPROVER_PK_MISMATCH"),
    # first real approval (ix 0, approver 1) → still active (1 < 2)
    ("approve", {"name": "satoshi", "id": "0", "ix": "0", "pubkey_hex": APR1_PK,
                 "signature": apr_sig(APR1, "satoshi", "0", "0"),
                 "expires_at": str(EXPIRES)}, "ok"),
    ("get_proposal", {"name": "satoshi", "id": "0"}, "active"),
    # double-approve → rejected
    ("approve", {"name": "satoshi", "id": "0", "ix": "0", "pubkey_hex": APR1_PK,
                 "signature": apr_sig(APR1, "satoshi", "0", "0"),
                 "expires_at": str(EXPIRES)}, "ERR_ALREADY_APPROVED"),
    # second approval (ix 1, approver 2) → threshold hit → approved
    ("approve", {"name": "satoshi", "id": "0", "ix": "1", "pubkey_hex": APR2_PK,
                 "signature": apr_sig(APR2, "satoshi", "0", "1"),
                 "expires_at": str(EXPIRES)}, "ok"),
    ("get_proposal", {"name": "satoshi", "id": "0"}, "approved"),
    # owner executes → transfer fires (mock prints receipt)
    ("execute", {"name": "satoshi", "id": "0",
                 "signature": owner_sig("execute:satoshi:0", 34),
                 "expires_at": str(EXPIRES), "nonce": "34"}, "ok"),
    ("get_proposal", {"name": "satoshi", "id": "0"}, "executed"),
    # re-execute → not approved anymore (st=executed)
    ("execute", {"name": "satoshi", "id": "0",
                 "signature": owner_sig("execute:satoshi:0", 35),
                 "expires_at": str(EXPIRES), "nonce": "35"}, "ERR_NOT_APPROVED"),

    # ── Phase 2.5: governance via EVENT auth (ev-branch of auth-owner) ──
    # funded wallet via event create (nonce 56), then the full
    # set_approvers → propose → execute cycle signed as kind-37500 events
    ("create_wallet", dict({"name": "evgov"},
                           **gov_event(SK, PK, "create_wallet:evgov", 56, EXPIRES, CONTRACT)),
     "ok", 500000000000000000000000),
    ("set_approvers", dict({"name": "evgov", "pks": f"{APR1_PK},{APR2_PK}", "thr": "2"},
                           **gov_event(SK, PK, "set_approvers:evgov", 50, EXPIRES, CONTRACT)),
     "ok"),
    # event path hits the same threshold validation
    ("set_approvers", dict({"name": "evgov", "pks": f"{APR1_PK},{APR2_PK}", "thr": "3"},
                           **gov_event(SK, PK, "set_approvers:evgov", 51, EXPIRES, CONTRACT)),
     "ERR_THRESHOLD_INVALID"),
    ("propose", dict({"name": "evgov", "pexp": str(EXPIRES), "am": "50000000000000000000000",
                      "rc": "rita.test.near"},
                     **gov_event(SK, PK, "propose:evgov:0", 52, EXPIRES, CONTRACT)),
     "ok"),
    ("get_proposal", {"name": "evgov", "id": "0"}, "active"),
    # execute via event before approvals → state machine rejects
    ("execute", dict({"name": "evgov", "id": "0"},
                     **gov_event(SK, PK, "execute:evgov:0", 53, EXPIRES, CONTRACT)),
     "ERR_NOT_APPROVED"),
    # wrong action tag (names a different wallet) → rejected at auth layer
    ("execute", dict({"name": "evgov", "id": "0"},
                     **gov_event(SK, PK, "execute:other:0", 54, EXPIRES, CONTRACT)),
     "ERR_EVENT_ACTION"),
    # approvals are per-approver legacy schnorr (by design)
    ("approve", {"name": "evgov", "id": "0", "ix": "0", "pubkey_hex": APR1_PK,
                 "signature": apr_sig(APR1, "evgov", "0", "0"),
                 "expires_at": str(EXPIRES)}, "ok"),
    ("approve", {"name": "evgov", "id": "0", "ix": "1", "pubkey_hex": APR2_PK,
                 "signature": apr_sig(APR2, "evgov", "0", "1"),
                 "expires_at": str(EXPIRES)}, "ok"),
    ("get_proposal", {"name": "evgov", "id": "0"}, "approved"),
    # the payoff: execute signed as a nostr event → transfer fires
    ("execute", dict({"name": "evgov", "id": "0"},
                     **gov_event(SK, PK, "execute:evgov:0", 55, EXPIRES, CONTRACT)),
     "ok"),
    ("get_proposal", {"name": "evgov", "id": "0"}, "executed"),
    # pause is a kill-switch for owner-gated actions in BOTH dialects
    ("pause", gov_event(SK, PK, "pause", 0, EXPIRES, CONTRACT, content="hold"),
     "ok"),
    ("set_approvers", dict({"name": "evgov", "pks": f"{APR1_PK},{APR2_PK}", "thr": "2"},
                           **gov_event(SK, PK, "set_approvers:evgov", 57, EXPIRES, CONTRACT)),
     "ERR_PAUSED"),
    ("unpause", gov_event(SK, PK, "unpause", 58, EXPIRES, CONTRACT),
     "ok"),

    # ── regression: tk routing must never reach a phantom contract ──
    # (2026-09-02 live catch: absent tk stored "nil", execute routed the
    # payout to an ft_transfer promise on account "nil" — mock used to
    # swallow unknown-account FnCalls; now it traps, vector pins it)
    ("propose", dict({"name": "evgov", "pexp": str(EXPIRES), "am": "10000000000000000000000",
                      "rc": "rita.test.near", "tk": "phantom.kampy.testnet"},
                     **gov_event(SK, PK, "propose:evgov:1", 59, EXPIRES, CONTRACT)),
     "ok"),
    ("approve", {"name": "evgov", "id": "1", "ix": "0", "pubkey_hex": APR1_PK,
                 "signature": apr_sig(APR1, "evgov", "1", "0"),
                 "expires_at": str(EXPIRES)}, "ok"),
    ("approve", {"name": "evgov", "id": "1", "ix": "1", "pubkey_hex": APR2_PK,
                 "signature": apr_sig(APR2, "evgov", "1", "1"),
                 "expires_at": str(EXPIRES)}, "ok"),
    # mock parity: unknown-token FnCall receipt fails like on-chain
    ("execute", dict({"name": "evgov", "id": "1"},
                     **gov_event(SK, PK, "execute:evgov:1", 60, EXPIRES, CONTRACT)),
     "MOCK-CHAIN-FAILURE: promise FnCall to unknown account 'phantom.kampy.testnet'"),
]

for name, args, expect, *rest in steps:
    dep = rest[0] if rest else 0
    print(json.dumps({"method": name, "args": args, "expect": expect, "deposit": dep}))
