#!/usr/bin/env python3
"""rlm-factcheck — cross-examine the agent's own claims against ground truth.

Scans recent lessons (traces) + tactics.txt for signature claims about
builtins, checks them against the TRUE signatures extracted from
src/helpers.rs, and writes:
  data/rlm/dream/factcheck.json       — every claim with a verdict
  data/rlm/dream/facts-corrections.txt — WRONG beliefs, phrased as
        corrections; the tactics mutator injects this next cycle so the
        agent rewrites its own advice around its proven errors.
Verdicts: right / wrong / no-claim. Pure stdlib, no LLM.
"""
import json, os, re, glob, time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DREAM = os.path.join(REPO, "data", "rlm", "dream")
TRACES = os.path.join(REPO, "data", "rlm", "traces")
TACTICS = os.path.join(REPO, "scripts", "rlm-tasks", "tactics.txt")
N_LESSONS = 40


def true_sigs():
    """fn -> param list, from helpers.rs doc lines: "fn" => "(fn a b) — ..."."""
    sigs = {}
    for ln in open(os.path.join(REPO, "src", "helpers.rs")):
        m = re.match(r'\s*"([^"]+)"(?:\s*\|\s*"[^"]+")*\s*=>\s*"\(([^)\s]+)\s+([^)]*)\)', ln)
        if m:
            fn = m.group(2)
            params = m.group(3).split()
            sigs.setdefault(fn, [p for p in params if p not in ("...",)])
    return sigs


def load_lessons():
    out = []
    files = sorted(glob.glob(os.path.join(TRACES, "*", "*.json")),
                   key=os.path.getmtime) + \
            glob.glob(os.path.join(TRACES, "*.json"))
    for p in sorted(files, key=os.path.getmtime)[-400:]:
        try:
            w = json.loads(open(p, "rb").read().decode().replace("\\", "\\\\"))
        except Exception:
            continue
        if w.get("lesson"):
            out.append(w["lesson"])
    return out[-N_LESSONS:]


ALIAS = {  # canonical type/family map — semantic, not lexical, matching
    "s": "str", "str": "str", "string": "str", "t": "str", "s0": "str",
    "original": "str", "source": "str", "input": "str",
    "l": "lst", "l1": "lst", "l2": "lst", "lst": "lst", "list": "lst",
    "xs": "lst", "arr": "lst", "seq": "lst",
    "sep": "sep", "separator": "sep", "delim": "sep", "delimiter": "sep",
    "new": "new", "replacement": "new", "repl": "new",
    "old": "old",
    "n": "num", "num": "num", "x": "num", "i": "num", "k": "num",
}


def canon(tok):
    return ALIAS.get(tok, tok)


def param_index_of(phrase_tokens, params):
    """Best param index the noun-phrase refers to, or None.

    Forward-first: in English noun phrases the modifier precedes the
    head ("the REPLACEMENT string" — 'replacement'→new carries the
    claim, not 'string'→s), so the earliest mapping wins.
    """
    STOP = {"the", "a", "an"}
    for tok in phrase_tokens:
        if tok in STOP:
            continue
        c = canon(tok)
        for i, p in enumerate(params):
            if c == canon(p):
                return i
    return None


def claims_in(text, sigs):
    """Yield (fn, verdict, evidence) for signature claims in text."""
    # pattern A: "(fn x y ...)" paren forms — arg order stated directly
    for m in re.finditer(r"\(([a-z][a-z0-9?!*-]*)((?:\s+[^\s()]+){0,6})\)", text):
        fn, rest = m.group(1), m.group(2).split()
        if fn in sigs and rest:
            if rest == sigs[fn][:len(rest)]:
                yield fn, "right", m.group(0)
            else:
                # only flag as wrong if the args LOOK like the true params
                # (permuted) — unrelated words (variable names) aren't claims
                if sorted(rest) == sorted(sigs[fn][:len(rest)]):
                    yield fn, "wrong", m.group(0)
    # pattern B: "X [Y] as the first argument" — noun phrase vs true order
    for m in re.finditer(
            r"([a-z][a-z0-9?!*-]{2,})\s+(?:as\s+)?(?:the\s+)?(?:first|1st)\s+"
            r"(?:argument|arg|param|parameter)|(?:first|1st)\s+(?:argument|arg)"
            r"\s+is\s+([a-z][a-z0-9?!*-]{2,})", text):
        tok = m.group(1) or m.group(2)
        # noun phrase = this token + up to 2 preceding tokens
        prior = re.findall(r"[a-z][a-z0-9?!*-]{2,}", text[:m.start()])
        phrase = ([t for t in prior[-2:]] if prior else []) + [tok]
        # find which fn is being discussed (nearest preceding fn mention)
        fns = [(mm.start(), mm.group(0)) for mm in
               re.finditer(r"[a-z][a-z0-9?!*-]{2,}", text[:m.start()])]
        fn = None
        for _, f in reversed(fns):
            if f in sigs and sigs[f]:
                fn = f
                break
        if fn:
            idx = param_index_of(phrase, sigs[fn])
            if idx is not None:
                truth_first = sigs[fn][0]
                yield fn, ("right" if idx == 0 else "wrong"), \
                    f"'{' '.join(phrase)}' refers to arg {idx + 1} of " \
                    f"({fn} {' '.join(sigs[fn])}), not '{truth_first}'"


def main():
    sigs = true_sigs()
    sources = [("lesson", l) for l in load_lessons()]
    if os.path.exists(TACTICS):
        sources.append(("tactics", open(TACTICS).read()))

    rows = []
    for src, text in sources:
        for fn, verdict, ev in claims_in(text, sigs):
            rows.append({"source": src, "fn": fn, "verdict": verdict,
                         "claim": ev, "truth": f"({fn} {' '.join(sigs[fn])})",
                         "text": text[:120]})
    # dedup (fn, verdict, claim)
    seen, uniq = set(), []
    for r in rows:
        k = (r["fn"], r["verdict"], r["claim"])
        if k not in seen:
            seen.add(k)
            uniq.append(r)

    wrong = [r for r in uniq if r["verdict"] == "wrong"]
    json.dump({"updated": time.strftime("%Y-%m-%d %H:%M:%S"),
               "claims": uniq},
              open(os.path.join(DREAM, "factcheck.json"), "w"), indent=1)

    if wrong:
        lines = ["# your RECENT WRONG beliefs — machine-verified against the "
                 "builtin docs. Correct these in tactics:", ""]
        for r in wrong:
            lines.append(f"- WRONG: {r['claim']}  TRUE: {r['truth']}")
        open(os.path.join(DREAM, "facts-corrections.txt"), "w").write("\n".join(lines))
    elif os.path.exists(os.path.join(DREAM, "facts-corrections.txt")):
        os.remove(os.path.join(DREAM, "facts-corrections.txt"))

    print(f"factcheck: {len(uniq)} claims "
          f"({sum(1 for r in uniq if r['verdict']=='right')} right, "
          f"{len(wrong)} wrong) — "
          + ("; corrections written" if wrong else "no corrections needed"))


if __name__ == "__main__":
    main()
