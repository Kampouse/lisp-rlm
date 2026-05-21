## Examples

### `wallet.lisp` — Ed25519 Smart Contract Wallet

A minimal NEAR wallet contract written in lisp-rlm. Uses Ed25519 keys for authentication with a session key flow for gasless delegation.

**Methods:**

| Method | Type | Args | Description |
|--------|------|------|-------------|
| `w_init` | call | 32 bytes Ed25519 pubkey | Initialize wallet with owner public key |
| `w_public_key` | view | — | Returns owner pubkey as hex string |
| `w_nonce` | view | — | Returns current execution nonce as hex |
| `w_add_session_key` | call | session_pk(32B) + expiry(4B u32) | Register a session key that expires at block height |
| `w_execute_session` | call | session_pk(32B) + nonce(4B) + signed_data + signature(64B) | Execute action with session key. Verifies Ed25519 signature over `nonce ++ signed_data`. Increments nonce and burns session key on success. |

**Storage layout:**
- `"pk"` → 32 bytes Ed25519 public key
- `"nonce"` → 4 bytes u32 LE execution counter
- `"sk" + session_pk` → 4 bytes u32 LE expiry block height

**Session key flow:**
1. Owner calls `w_add_session_key` with a temporary Ed25519 public key + expiry block height
2. Session signer creates signature over `nonce || action_data`
3. Anyone calls `w_execute_session` with the session pubkey, nonce, action data, and signature
4. Contract verifies signature, checks nonce, increments nonce, burns session key

**Compiled size:** ~9.5 KB (optimized with `wasm-opt -Oz`)

---

### `factory.lisp` — Wallet Factory (Traditional Pattern)

Deploys wallet contracts on subaccounts. Stores the wallet WASM once, then creates subaccounts and deploys the code.

**Methods:**

| Method | Type | Args | Description |
|--------|------|------|-------------|
| `f_init` | call | wallet WASM bytes | Store wallet code in factory storage (call once) |
| `f_create` | call | suffix_len(4B u32) + suffix_bytes + pk_bytes(32B) | Create `{suffix}.{factory}` subaccount, deploy wallet, initialize with public key. Attaches deposited NEAR to new account. |

**Flow:**
1. Deploy factory contract to account (e.g. `factory.testnet`)
2. Call `f_init` with compiled wallet WASM as input
3. For each new wallet, call `f_create` with suffix + owner pubkey + attached deposit (≥0.1 NEAR)
4. Factory creates subaccount, deploys wallet code, calls `w_init`

**Cost per wallet:** ~0.13 NEAR (0.1 NEAR deposit + ~0.03 NEAR gas)

**Compiled size:** ~2.7 KB

---

### Compiling

```bash
./target/release/near-compile examples/wallet.lisp wallet.wasm
./target/release/near-compile examples/factory.lisp factory.wasm
```

Optional size optimization:
```bash
wasm-opt -Oz wallet.wasm -o wallet_opt.wasm
```
