# ft — NEP-141 probe fixture for the escrow leg

STATIC fixture (hand-written, not generator-emitted): the minimum NEP-141
surface needed to prove the escrow's two-hop deposit by execution.
Not a product — the real deployment reuses the platform FT.

## Exports

| export | what |
|--------|------|
| `mint` | test faucet: credit `to` (fixture only) |
| `register` | storage stamp (fixture only) |
| `ft_balance_of` | view |
| `ft_transfer` | one-hop debit/credit |
| `ft_transfer_call` | debit sender → `promise_create` receiver `ft_on_transfer {amount, msg}` → `promise_then` SELF `ft_resolve` |
| `ft_resolve` | empty promise result ⇒ refund = re-credit sender |

## Executed (make escrow)

- `scenarios/lisp/ft-probe` 5/5: mint 5000 → `ft_transfer_call` 1000
  into escrow with `msg = trader.test.near` → FT balance 4000,
  escrow `e:bal:trader = 1000`, `e:in = 1000`.
- `scenarios/lisp/ft-refund` 3/3: `ft_transfer_call` at a contract with
  **no** `ft_on_transfer` → failed receipt → `ft_resolve` refunds →
  sender ends at exactly 5000. The bounce path, proven.
