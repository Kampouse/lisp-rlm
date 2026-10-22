# Launch tab (bond-curve launchpad) — wiring + deploy log

## Live launch pair (2026-10-21)
- **bond4.clmm5-51fe5a.testnet** — curve, bond_curve_v1 HEAD build (50,419 B,
  source unchanged since mock-verified `e905be56` era; deploy `6LLLxM3A…`,
  init `8vuM91ah…`). Config = bond2's: qt=wnear, pool=**pool9** (FUTURE pool,
  pre-pinned; create + deploy it before graduation!), sup=1e27, v=3e25,
  g=1e24 (=1 wnear target), lo=-8 hi=8.
- bond2 is graduated+frozen (grad=1) — kept as the reference pair.

## Curve ABI (all money math u128 string domain, wnear = 24 decimals!)
- `state` → `qt:…:pool:…:sup:…:x:…:y:…:r:…:sold:…:grad:…` — **NO `g` key!**
  Get the raise target from `view_state` key `C|G` (UI does both).
- `quote_buy {q}` → raw token out; `quote_sell {t}` → raw wnear out.
- buy = `wnear.ft_transfer_call` → curve, msg `{"a":"buy"}`, attach = q raw.
- sell = curve `sell {t}` direct (internal b: ledger, no FT registration).
- `ft_balance_of` = the b: ledger. Post-grad: buys REFUND (ft_on_transfer
  returns amt), sells reject ERR_GRAD.
- UNITS: token display = raw/1e15; **wNEAR display = raw/1e24** — never
  human() a wnear amount (1e9× error).

## UI
- Third tab "Launch" (tabLaunch/viewLaunch): price (x/y), lime progress bar
  r/C|G, buy/sell cards with live quotes (300ms debounce), MAX fill.
- Buy/Sell via walletTx (seam + wallet both work — both live-tested).

## Fund-sweep lessons (sweep #3, funding the pair)
- near CLI v4 txs SILENTLY DON'T LAND (deprecated RPC) — only its `state`
  queries are trustworthy. All sends via naj high-level API
  (`acct.sendMoney`, `signAndSendTransaction`) on fastnear.
- create.js in swap-ui: createAccount + transfer + addKey(**fullAccessKey**)
  — an FC-only key TRAPS the account (can't deploy/delete). pool8 (29 N) and
  bond3.clmm5 (9 N) are trapped this way — names burned, don't reuse.
- `bond3.e.…` names: the `.e` subdomain has a separate registrar — plain
  sub-accounts only.
- Contract accounts floor-refuse drains (storage stake) — plain wallets
  only are sweepable (xcross-9f3, registry-wot, registry-variant-c, wtw3,
  pwallet1 drained to ~floors; lff2 must keep ≥27 N = sponsor law).
- Master (clmm5-51fe5a.testnet) is the sub-account registrar funder; keep
  it ≥ a few N.
