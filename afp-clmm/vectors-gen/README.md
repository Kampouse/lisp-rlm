# vectors-gen — machine-checked accrual fixtures

Bridge between the proven layer and the port twins, WITHOUT touching
any port file. Three independent sources, one arithmetic:

1. `../isabelle/CLMM_Fee_Vectors.thy` — pin theorems (structural +
   numeric) over the exact `accruef`/`prodsum` definitions that
   Aggregate_Accrual's bridge theorem quantifies over. Every pin is
   build-gated; 0 sorries.
2. `Accruef.sml` — Isabelle's own code-generator export of those
   definitions (native bigints via `Code_Target_Numeral`; the nat type
   is opaque, so the converters `integer_of_nat`/`nat_of_integer` are
   exported in the module signature — use them, the constructors are
   hidden).
3. `crosscheck.py` — an independent Python mirror of the fold, written
   from the ISAR doc comment in the lisp twin (NOT from the SML), so
   any export/generator bug shows as a diff. Exit 1 on mismatch.

## Regenerate the fixtures

One-time prerequisite (the CLMM session does not depend on HOL-Library):

    /Users/asil/Isabelle2025-2.app/bin/isabelle build HOL-Library

The theory is NOT registered in the session ROOT (Jean's local edit is
never staged by the campaign). Load it ad hoc — pass the whole local
import closure with `-f`; positional theory names resolve against the
private Draft dir in /tmp and fail:

    cd afp-clmm
    /Users/asil/Isabelle2025-2.app/bin/isabelle process_theories \
      -l HOL-Library \
      -f isabelle/Aggregate_Book.thy \
      -f isabelle/Aggregate_Accrual.thy \
      -f isabelle/CLMM_Fee_Vectors.thy \
      -O Aggregate_Book Aggregate_Accrual CLMM_Fee_Vectors

This re-proves the pins and runs the theory's `export_code`, which
rewrites `Accruef.sml` (exit 0; the "Legacy feature!" warning about
`export_code` with a file argument is expected).

Then evaluate — poly prints a banner + signature before the JSON, so
keep the sed filter (the JSON starts at the first "["):

    /Users/asil/Isabelle2025-2.app/contrib/polyml-5.9.2-2/arm64-darwin/poly \
      < run_vectors.sml 2>/dev/null | sed -n '/^\[/,$p' > accrual-vectors.json

## Cross-check (independent mirror)

    python3 crosscheck.py < accrual-vectors.json

Current status: 5/5 rows match.

## Consuming in smoke scenarios

`accrual-vectors.json` rows are `(g, owed_one, owed_g, undivided)` —
the exact numbers a twin accrual step must produce for growth
sequence `[g]` from checkpoint 0 under cap 1 / cap = g. Feed them to
the scenario runner as expected outputs.

## Scope (honest)

This pins the FOLD the theorems bound. It does not verify the twins'
call-path wiring — the transcription proof remains open work in the
port lane (Jean's).
