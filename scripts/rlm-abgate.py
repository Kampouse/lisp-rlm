#!/usr/bin/env python3
"""rlm-abgate — A/B gate for agent-proposed custom actions.

Lifecycle: candidates/action-proposal.lisp (validated by the dream task)
→ install into scripts/rlm-tasks/custom-actions.lisp → alternating
cycles ON/OFF (weak tasks t2/t3/t5 solve-rate is the metric) → after
enough paired cycles, promote (keep + ledger) or reject (uninstall +
ledger). State: data/rlm/dream/ab.json. No LLM, no human.
"""
import json, os, subprocess, sys, time, glob, re

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CAND = os.path.join(REPO, "candidates", "action-proposal.lisp")
INSTALLED = os.path.join(REPO, "scripts", "rlm-tasks", "custom-actions.lisp")
AB = os.path.join(REPO, "data", "rlm", "dream", "ab.json")
LEDGER = os.path.join(REPO, "data", "rlm", "dream", "ledger.jsonl")
TRACES = os.path.join(REPO, "data", "rlm", "traces")
WEAK = ("t2_reverse", "t3_fib", "t5_vowels")
MIN_CYCLES = 6


def log(ev):
    with open(LEDGER, "a") as f:
        f.write(json.dumps({"ts": time.time(), **ev}) + "\n")


def load():
    return json.load(open(AB)) if os.path.exists(AB) else {"phase": "idle"}


def save(st):
    json.dump(st, open(AB, "w"), indent=1)


def weak_solves_since(ts):
    n = solved = 0
    for p in glob.glob(os.path.join(TRACES, "*", "*.json")):
        m = re.match(r".+-(\d{6})", os.path.basename(p))
        if not m or m.group(1) < time.strftime("%H%M%S", time.localtime(ts)):
            continue
        base = os.path.basename(p)
        if not any(base.startswith(w) for w in WEAK):
            continue
        try:
            w = json.loads(open(p).read().replace("\\", "\\\\"))
        except Exception:
            continue
        n += 1
        solved += 1 if w.get("completed") else 0
    return solved, n


def main():
    st = load()

    if st["phase"] == "idle":
        if not os.path.exists(CAND):
            return  # nothing proposed
        src = open(CAND).read()
        open(INSTALLED, "w").write(src)
        os.remove(CAND)
        st = {"phase": "on", "cycles_on": 0, "cycles_off": 0,
              "solved_on": 0, "n_on": 0, "solved_off": 0, "n_off": 0,
              "started": time.time()}
        save(st)
        log({"event": "ab_install", "action": src[:80]})
        print("abgate: candidate INSTALLED (arm ON)")
        return

    # alternate arm each call (each cycle calls abgate once)
    arm = "on" if st.get("last_arm") != "on" else "off"
    st["last_arm"] = arm
    st[f"cycles_{arm}"] += 1

    if arm == "off" and os.path.exists(INSTALLED):
        os.rename(INSTALLED, INSTALLED + ".held")
        # leave a stub, NOT a gap: every task file hard-loads this path
        # and a missing file kills the whole cycle at load (lost 10:45-11:30)
        open(INSTALLED, "w").write(";; stub — held by abgate (off arm)\n")
    elif arm == "on" and os.path.exists(INSTALLED + ".held"):
        os.replace(INSTALLED + ".held", INSTALLED)  # atomically overwrites stub

    s, n = weak_solves_since(time.time() - 60 * 11)
    st[f"solved_{arm}"] += s
    st[f"n_{arm}"] += n
    save(st)

    if st["cycles_on"] >= MIN_CYCLES and st["cycles_off"] >= MIN_CYCLES:
        def rate(k_s, k_n):
            return st[k_s] / st[k_n] if st[k_n] else 0.0
        on, off = rate("solved_on", "n_on"), rate("solved_off", "n_off")
        if on > off + 0.05 and st["n_on"] >= 8:
            log({"event": "ab_promote", "rate_on": round(on, 3),
                 "rate_off": round(off, 3)})
            print(f"abgate: PROMOTED (on {on:.0%} vs off {off:.0%})")
            if os.path.exists(INSTALLED + ".held"):
                os.rename(INSTALLED + ".held", INSTALLED)
            save({"phase": "idle"})  # ready for next candidate
        else:
            log({"event": "ab_reject", "rate_on": round(on, 3),
                 "rate_off": round(off, 3)})
            print(f"abgate: REJECTED (on {on:.0%} vs off {off:.0%})")
            for p in (INSTALLED, INSTALLED + ".held"):
                if os.path.exists(p):
                    os.remove(p)
            save({"phase": "idle"})
    else:
        print(f"abgate: arm={arm} on {st['solved_on']}/{st['n_on']} "
              f"off {st['solved_off']}/{st['n_off']} "
              f"(cycles {st['cycles_on']}on/{st['cycles_off']}off)")


if __name__ == "__main__":
    main()
