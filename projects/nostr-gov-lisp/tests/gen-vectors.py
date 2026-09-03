#!/usr/bin/env python3
"""gen-vectors.py — v2 governance vectors (typed proposals, admin set).

v2 semantics under test:
  - admin actions (create/propose/execute/pause) are EVENT-auth only;
    the legacy single-key dialect returns ERR_EV_REQUIRED
  - admin set = approvers of implicit wallet "gov" (bootstrap =
    owner_npub0, thr 1); wallets are BORN with their approver set
  - set rotation is approver-gated (appr proposals), including the
    admin set itself (self-governance via wallet "gov")
  - pause = any admin; unpause = M-of-N "unp" proposal on "gov";
    while paused only the recovery path may run
"""
import json, sys
sys.path.insert(0, ".")
from bip340 import gov_event, sign, sign_event, sha, i2b, mul, b2i

CONTRACT = "escrow.test.near"
SK = bytes([0xAA] * 32)          # bootstrap admin (owner_npub0)
PK = i2b(mul(b2i(SK))[0]).hex()
OSK = bytes([0xEE] * 32)         # a NON-admin key
OPK = i2b(mul(b2i(OSK))[0]).hex()
APR1 = bytes([0xCC] * 32)        # wallet approvers
APR2 = bytes([0xDD] * 32)
APR1_PK = i2b(mul(b2i(APR1))[0]).hex()
APR2_PK = i2b(mul(b2i(APR2))[0]).hex()
TS = int(sys.argv[1]) if len(sys.argv) > 1 else 1787000000_000000000
EXPIRES = TS + 3600_000000000
LATE = TS - 3600 * 10**9
DEP = 500000000000000000000000

def owner_msg(action, nonce):
    return (f"expires {EXPIRES}.000000000: {action} | nonce: {nonce} "
            f"| contract: {CONTRACT}").encode()

def owner_sig(action, nonce):
    return sign(SK, sha(owner_msg(action, nonce))).hex()

def apr_sig(sk, name, pid, ix):
    m = (f"expires {EXPIRES}.000000000: approve:{name}:{pid}:{ix} "
         f"| contract: {CONTRACT}").encode()
    return sign(sk, sha(m)).hex()

def ev(action, nonce, exp=EXPIRES, contract=CONTRACT, sk=SK, pk=PK, **kw):
    return gov_event(sk, pk, action, nonce, exp, contract, **kw)

def badflip(evd):
    """flip last char of the event sig"""
    d = dict(evd)
    s = list(d["sig"])
    s[-1] = "0" if s[-1] != "0" else "1"
    d["sig"] = "".join(s)
    return d


def pk_family(seed, n):
    """n distinct pks from one seed (mirrors bip340 mul)."""
    return ",".join(i2b(mul(b2i(bytes([seed] * 32) + i.to_bytes(1, "big")))[0]).hex() for i in range(n))

def approval_event(sk, pk, contract, wallet, pid, ix, exp, action="approve",
                   kind=37500, created_at=1, ct_override=None, sig_override=None):
    """Kind-37500 gasless approval event (FE buildApprovalEvent shape)."""
    msg = (f"expires {exp}.000000000: approve:{wallet}:{pid}:{ix} "
           f"| contract: {contract}")
    tags = [["contract", contract], ["wallet", wallet],
            ["proposal", str(pid)], ["approver", str(ix)], ["action", action]]
    ct = ct_override if ct_override is not None else msg
    sig = sign_event(sk, pk, created_at, kind, tags, ct)
    if sig_override is not None:
        sig = sig_override
    return {"pk": pk, "cat": str(created_at), "kind": str(kind),
            "tags": json.dumps(tags, separators=(",", ":")),
            "ct": ct, "sig": sig}

steps = [
    # ── 0. init & public self-test ──────────────────────────────────
    ("init", {"npub": PK}, "ok"),
    ("test_verify_nostr", {"message": "test", "pubkey_hex": PK,
                           "sig_hex": sign(SK, sha(b"test")).hex()}, "ok"),

    # ── 1. legacy dialect on admin paths is CLOSED (v2) ────────────
    ("create_wallet", {"name": "satoshi", "signature": owner_sig("create_wallet:satoshi", 7),
                       "expires_at": str(EXPIRES), "nonce": "7"}, "ERR_EV_REQUIRED"),
    ("pause", {"signature": sign(SK, sha(
        f"expires {EXPIRES}.000000000: pause | contract: {CONTRACT}".encode())).hex(),
               "expires_at": str(EXPIRES)}, "ERR_EV_REQUIRED"),

    # ── 2. event auth: wallets born with approvers ─────────────────
    ("create_wallet", dict({"name": "evented", "pks": f"{APR1_PK},{APR2_PK}", "thr": "2"},
                           **ev("create_wallet:evented", 20)), "ok", DEP),
    ("create_wallet", dict({"name": "evented", "pks": f"{APR1_PK},{APR2_PK}", "thr": "2"},
                           **ev("create_wallet:evented", 21)), "ERR_WALLET_EXISTS", DEP),
    ("create_wallet", dict({"name": "nofund", "pks": APR1_PK, "thr": "1"},
                           **ev("create_wallet:nofund", 22)), "ERR_STORAGE_DEPOSIT"),
    ("create_wallet", dict({"name": "late", "pks": APR1_PK, "thr": "1"},
                           **ev("create_wallet:late", 23, exp=LATE)), "ERR_SIG_EXPIRED"),
    ("create_wallet", dict({"name": "badsig", "pks": APR1_PK, "thr": "1"},
                           **badflip(ev("create_wallet:badsig", 24))), "ERR_EVENT_SIG_INVALID"),
    ("create_wallet", dict({"name": "wrongkind", "pks": APR1_PK, "thr": "1"},
                           **ev("create_wallet:wrongkind", 25, kind=1)), "ERR_EVENT_KIND"),
    ("create_wallet", dict({"name": "mismatch", "pks": APR1_PK, "thr": "1"},
                           **ev("create_wallet:other", 26)), "ERR_EVENT_ACTION"),
    ("create_wallet", dict({"name": "wrongct", "pks": APR1_PK, "thr": "1"},
                           **ev("create_wallet:wrongct", 27, contract="wrong.testnet")),
     "ERR_EVENT_CONTRACT"),
    # non-admin nostr key → rejected
    ("create_wallet", dict({"name": "intruder", "pks": APR1_PK, "thr": "1"},
                           **ev("create_wallet:intruder", 28, sk=OSK, pk=OPK)),
     "ERR_EVENT_PK_MISMATCH"),
    # the governance wallet name is reserved
    ("create_wallet", dict({"name": "gov", "pks": APR1_PK, "thr": "1"},
                           **ev("create_wallet:gov", 29)), "ERR_NAME_RESERVED", DEP),
    # approver-set validation at birth
    ("create_wallet", dict({"name": "noappr", "thr": "1"},
                           **ev("create_wallet:noappr", 31)), "ERR_APPROVERS_EMPTY", DEP),
    ("create_wallet", dict({"name": "badthr", "pks": f"{APR1_PK},{APR2_PK}", "thr": "3"},
                           **ev("create_wallet:badthr", 32)), "ERR_THRESHOLD_INVALID", DEP),
    # nonce replay (20 consumed by the successful create)
    ("create_wallet", dict({"name": "replay", "pks": APR1_PK, "thr": "1"},
                           **ev("create_wallet:replay", 20)), "ERR_NONCE_ALREADY_USED", DEP),
    ("get_wallet", {"name": "ghost"}, ""),

    # ── 3. G-15 jump-on-high (sparse signers never brick) ──────────
    ("create_wallet", dict({"name": "jumper", "pks": APR1_PK, "thr": "1"},
                           **ev("create_wallet:jumper", 5000)), "ok", DEP),
    ("create_wallet", dict({"name": "jumper", "pks": APR1_PK, "thr": "1"},
                           **ev("create_wallet:jumper", 30)), "ERR_NONCE_TOO_LOW", DEP),
    ("create_wallet", dict({"name": "jumper2", "pks": APR1_PK, "thr": "1"},
                           **ev("create_wallet:jumper2", 5001)), "ok", DEP),
    ("create_wallet", dict({"name": "jumper3", "pks": APR1_PK, "thr": "1"},
                           **ev("create_wallet:jumper3", 5000)), "ERR_NONCE_TOO_LOW", DEP),

    # ── 4. payout lifecycle (wallet evented: APR1,APR2 / thr 2) ────
    ("propose", dict({"name": "evented", "pexp": str(EXPIRES), "am": "10000000000000000000000",
                      "rc": "rich.test.near"},
                     **ev("propose:evented:5100", 5100)), "ok"),
    ("approve", {"name": "evented", "id": "5100", "ix": "0", "pubkey_hex": OPK,
                 "signature": apr_sig(APR1, "evented", "5100", "0"),
                 "expires_at": str(EXPIRES)}, "ERR_APPROVER_PK_MISMATCH"),
    ("approve", {"name": "evented", "id": "9", "ix": "0", "pubkey_hex": APR1_PK,
                 "signature": apr_sig(APR1, "evented", "9", "0"),
                 "expires_at": str(EXPIRES)}, "ERR_PROPOSAL_NOT_FOUND"),
    # legacy execute is closed — but only AFTER the proposal is found
    ("execute", {"name": "evented", "id": "5100", "signature": owner_sig("execute:evented:0", 41),
                 "expires_at": str(EXPIRES), "nonce": "5101"}, "ERR_EV_REQUIRED"),
    ("execute", dict({"name": "evented", "id": "5100"}, **ev("execute:evented:5100", 5102)),
     "ERR_NOT_APPROVED"),
    ("approve", {"name": "evented", "id": "5100", "ix": "0", "pubkey_hex": APR1_PK,
                 "signature": apr_sig(APR1, "evented", "5100", "0"),
                 "expires_at": str(EXPIRES)}, "ok"),
    ("execute", dict({"name": "evented", "id": "5100"}, **ev("execute:evented:5100", 5103)),
     "ERR_NOT_APPROVED"),
    ("approve", {"name": "evented", "id": "5100", "ix": "1", "pubkey_hex": APR2_PK,
                 "signature": apr_sig(APR2, "evented", "5100", "1"),
                 "expires_at": str(EXPIRES)}, "ok"),
    ("execute", dict({"name": "evented", "id": "5100"}, **ev("execute:evented:5100", 5104)), "ok"),
    # already executed → not approved-for-execution
    ("execute", dict({"name": "evented", "id": "5100"}, **ev("execute:evented:5100", 5105)),
     "ERR_NOT_APPROVED"),
    # FT payout to a phantom token — receipt-failure parity line
    ("propose", dict({"name": "evented", "pexp": str(EXPIRES), "am": "10",
                      "rc": "rich.test.near", "tk": "phantom.kampy.testnet"},
                     **ev("propose:evented:5106", 5106)), "ok"),
    ("approve", {"name": "evented", "id": "5106", "ix": "0", "pubkey_hex": APR1_PK,
                 "signature": apr_sig(APR1, "evented", "5106", "0"),
                 "expires_at": str(EXPIRES)}, "ok"),
    ("approve", {"name": "evented", "id": "5106", "ix": "1", "pubkey_hex": APR2_PK,
                 "signature": apr_sig(APR2, "evented", "5106", "1"),
                 "expires_at": str(EXPIRES)}, "ok"),
    ("execute", dict({"name": "evented", "id": "5106"}, **ev("execute:evented:5106", 5107)),
     "MOCK-CHAIN-FAILURE: promise FnCall to unknown account 'phantom.kampy.testnet'"),

    # ── 5. approver rotation is approver-gated (wallet evented) ────
    ("propose", dict({"name": "evented", "act": "appr", "np": APR2_PK, "nt": "1",
                      "pexp": str(EXPIRES)},
                     **ev("propose:evented:5108", 5108)), "ok"),
    ("approve", {"name": "evented", "id": "5108", "ix": "0", "pubkey_hex": APR1_PK,
                 "signature": apr_sig(APR1, "evented", "5108", "0"),
                 "expires_at": str(EXPIRES)}, "ok"),
    ("execute", dict({"name": "evented", "id": "5108"}, **ev("execute:evented:5108", 5109)),
     "ERR_NOT_APPROVED"),
    ("approve", {"name": "evented", "id": "5108", "ix": "1", "pubkey_hex": APR2_PK,
                 "signature": apr_sig(APR2, "evented", "5108", "1"),
                 "expires_at": str(EXPIRES)}, "ok"),
    ("execute", dict({"name": "evented", "id": "5108"}, **ev("execute:evented:5108", 5110)), "ok"),
    ("get_approvers", {"name": "evented"}, f'{{"thr":"1","pks":"{APR2_PK}"}}'),
    ("propose", dict({"name": "evented", "act": "appr", "np": "", "nt": "1",
                      "pexp": str(EXPIRES)},
                     **ev("propose:evented:5111", 5111)), "ERR_APPROVERS_EMPTY"),
    ("propose", dict({"name": "evented", "act": "appr", "np": f"{APR1_PK},{APR2_PK}",
                      "nt": "3", "pexp": str(EXPIRES)},
                     **ev("propose:evented:5112", 5112)), "ERR_THRESHOLD_INVALID"),
    ("propose", dict({"name": "evented", "act": "zzz", "pexp": str(EXPIRES)},
                     **ev("propose:evented:5113", 5113)), "ERR_ACTION_UNKNOWN"),

    # ── 6. pause / unpause (asymmetric breaker) ────────────────────
    ("pause", ev("pause", 5114, sk=OSK, pk=OPK), "ERR_EVENT_PK_MISMATCH"),
    ("pause", ev("pause", 5114), "ok"),
    ("is_paused", {}, "1"),
    ("create_wallet", dict({"name": "pausednow", "pks": APR1_PK, "thr": "1"},
                           **ev("create_wallet:pausednow", 5115)), "ERR_PAUSED", DEP),
    ("propose", dict({"name": "evented", "pexp": str(EXPIRES), "am": "1",
                      "rc": "rich.test.near"},
                     **ev("propose:evented:5116", 5116)), "ERR_PAUSED"),
    # unpause proposals only exist on the governance wallet
    ("propose", dict({"name": "evented", "act": "unp", "pexp": str(EXPIRES)},
                     **ev("propose:evented:5117", 5117)), "ERR_NOT_GOVERNANCE"),
    # recovery path: propose unp on gov (bootstrap admin set = PK, thr 1)
    ("propose", dict({"name": "gov", "act": "unp", "pexp": str(EXPIRES)},
                     **ev("propose:gov:5118", 5118)), "ok"),
    ("approve", {"name": "gov", "id": "5118", "ix": "0", "pubkey_hex": PK,
                 "signature": apr_sig(SK, "gov", "5118", "0"),
                 "expires_at": str(EXPIRES)}, "ok"),
    ("execute", dict({"name": "gov", "id": "5118"}, **ev("execute:gov:5118", 5119)), "ok"),
    ("is_paused", {}, "0"),

    # ── 7. admin-set rotation = self-governance on "gov" ───────────
    ("propose", dict({"name": "gov", "act": "appr", "np": OPK, "nt": "1",
                      "pexp": str(EXPIRES)},
                     **ev("propose:gov:5120", 5120)), "ok"),
    ("approve", {"name": "gov", "id": "5120", "ix": "0", "pubkey_hex": PK,
                 "signature": apr_sig(SK, "gov", "5120", "0"),
                 "expires_at": str(EXPIRES)}, "ok"),
    ("execute", dict({"name": "gov", "id": "5120"}, **ev("execute:gov:5120", 5121)), "ok"),
    ("get_approvers", {"name": "gov"}, f'{{"thr":"1","pks":"{OPK}"}}'),
    # the OLD admin is now powerless
    ("pause", ev("pause", 5122), "ERR_EVENT_PK_MISMATCH"),
    ("create_wallet", dict({"name": "oldadmin", "pks": APR1_PK, "thr": "1"},
                           **ev("create_wallet:oldadmin", 5123)), "ERR_EVENT_PK_MISMATCH", DEP),
    # the NEW admin runs the contract (pause + recovery cycle)
    ("pause", ev("pause", 5124, sk=OSK, pk=OPK), "ok"),
    ("is_paused", {}, "1"),
    ("propose", dict({"name": "gov", "act": "unp", "pexp": str(EXPIRES)},
                     **ev("propose:gov:5125", 5125, sk=OSK, pk=OPK)), "ok"),
    ("approve", {"name": "gov", "id": "5125", "ix": "0", "pubkey_hex": OPK,
                 "signature": apr_sig(OSK, "gov", "5125", "0"),
                 "expires_at": str(EXPIRES)}, "ok"),
    ("execute", dict({"name": "gov", "id": "5125"}, **ev("execute:gov:5125", 5126, sk=OSK, pk=OPK)),
     "ok"),
    ("is_paused", {}, "0"),
    ("get_version", {}, "2"),
    # ── 11. hardening: >64 approvers rejected (i64 bitmap limit) ────
    ("create_wallet", dict({"name": "toomany", "pks": pk_family(0x9F, 65), "thr": "2"},
                           **ev("create_wallet:toomany", 5128, sk=OSK, pk=OPK)), "ERR_APPROVERS_TOO_MANY", DEP),

    # ── 12. gasless kind-37500 approvals (relay flow) ───────────────
    ("create_wallet", dict({"name": "gasless", "pks": f"{APR1_PK},{APR2_PK}", "thr": "2"},
                           **ev("create_wallet:gasless", 5130, sk=OSK, pk=OPK)), "ok", DEP),
    ("get_wallet_count", {}, "ok"),
    ("propose", dict({"name": "gasless", "am": "1000000000000000000000000",
                      "rc": "bob.testnet", "pexp": str(EXPIRES)},
                     **ev("propose:gasless:5131", 5131, sk=OSK, pk=OPK)), "ok"),
    ("get_proposal_ids", {"name": "gasless"}, "5131"),
    ("get_proposal_message", {"name": "gasless", "id": "5131", "ix": "0", "exp": str(EXPIRES)},
     f"expires {EXPIRES}.000000000: approve:gasless:5131:0 | contract: {CONTRACT}"),
    # happy: approver 0 signs a 37500 event, watcher submits
    ("approve_with_event", approval_event(APR1, APR1_PK, CONTRACT, "gasless", "5131", "0", EXPIRES), "ok"),
    # replay of the SAME event: bitmap rejects, idempotence holds
    ("approve_with_event", approval_event(APR1, APR1_PK, CONTRACT, "gasless", "5131", "0", EXPIRES),
     "ERR_ALREADY_APPROVED"),
    # cancel action tag is not an approval
    ("approve_with_event", approval_event(APR2, APR2_PK, CONTRACT, "gasless", "5131", "1", EXPIRES,
                                          action="cancel"), "ERR_EVENT_ACTION"),
    # tampered content (routing says 1, content says 0)
    ("approve_with_event", approval_event(APR2, APR2_PK, CONTRACT, "gasless", "5131", "1", EXPIRES,
                                          ct_override=f"expires {EXPIRES}.000000000: approve:gasless:5131:0 | contract: {CONTRACT}"),
     "ERR_EVENT_CONTENT"),
    # wrong kind (plain note)
    ("approve_with_event", approval_event(APR2, APR2_PK, CONTRACT, "gasless", "5131", "1", EXPIRES,
                                          kind=1), "ERR_EVENT_KIND"),
    # garbage signature
    ("approve_with_event", approval_event(APR2, APR2_PK, CONTRACT, "gasless", "5131", "1", EXPIRES,
                                          sig_override="00" * 64), "ERR_EVENT_SIG_INVALID"),
    # wrong contract tag
    ("approve_with_event", approval_event(APR2, APR2_PK, "other.testnet", "gasless", "5131", "1",
                                          EXPIRES), "ERR_EVENT_CONTRACT"),
    # approver 1 completes the threshold via the event path
    ("approve_with_event", approval_event(APR2, APR2_PK, CONTRACT, "gasless", "5131", "1", EXPIRES), "ok"),
    ("get_proposal", {"name": "gasless", "id": "5131"}, "approved"),
]


for name, args, expect, *rest in steps:
    dep = rest[0] if rest else 0
    print(json.dumps({"method": name, "args": args, "expect": expect, "deposit": dep}))
