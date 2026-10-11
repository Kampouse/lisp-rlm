# zk-squad v2 — Sealed-Tally Threshold Voting (3PVSS blueprint)

Source: Baghery–Makri–Suryakari, *Packed Pre-Constructed PVSS for Randomness
Generation and E-Voting*, ePrint 2026/1119 (SCN 2026, full version with proofs).
Protocol: multi-candidate e-voting on Λ_indv (Fig. 7), binary-vote OR-proof
(App. 2.3), Λ_indv transcript (Fig. 4). This doc maps it to NEAR + the
deployed zk-squad layer. Goal (JP, 2026-10-10): *encode a yes/no after a
threshold, based on everyone who voted, at a said time* — sealed tally,
deadline close, quorum open, single-bit output.

## The protocol, compressed

Squad members are BOTH voters and talliers (self-tally, n members, quorum t).
ℓ = 1 (binary yes/no). Robustness bound from the paper: **n ≥ 2t + ℓ** →
n=8 tolerates t=3 corrupted in tallying.

**INIT:** members publish tallier keys PK_i (each = g^{SK_i}); agree on
commitment key h₀, deadline T₁, threshold t, roster root (zk-identity layer).

**BALLOT (member = dealer, Fig. 4 Share):**
- sample polynomial f of degree ≤ t+ℓ−1 with secret f(0)=s; bit v ∈ {0,1}
- encrypted shares: yᵢ = PK_i^{f(i)}  (i = 1..n)
- secret commitments: yⱼ = h₀^{f(j)}  (j = −(ℓ−1)..0)
- ballot ciphertext: U = G^{s+v}  ← the sealed vote
- π_3PVSS = (d, z(X)): polynomial Fiat-Shamir proof that all y's share one f
- π_U (App. 2.3): disjunctive Chaum-Pedersen — proves v ∈ {0,1} via
  log_G U = log_g C₀ ∨ log_G U = 1 + log_g C₀
- [our layer] zk-identity proof: member ∧ nullifier (deployed, tonight)

**VERIFY (Fig. 4 Verify):** degree-check z(X); recompute
d = H(y⃗, h₀^{z(j)}/yⱼ^d, PK_i^{z(i)}/yᵢ^d). Pure exponentiations.
Also π_U: 4 multiexp-shaped checks aᵢ = g^{rᵢ}·C₀^{dᵢ}, bᵢ = G^{rᵢ}·(U/G^{1−v})^{dᵢ}.

**CLOSE (after T₁):** talliers homomorphically multiply accepted shares
(Ȳᵢ = Π_k y_{i,k} — sealed Σ of randomizers), reconstruct S̄ = g^{Σs_k}
(Lagrange over t+ℓ decryption shares + DLEQ per share), compute
Ū* = Π_k U_k = G^{Σs+Σv}, then **V* = Ū*/S̄ = G^{Σv}** — and Σv ∈ {0..m}
comes out by table lookup. Σv ≥ t → YES, else NO.

Pre-constructability: commitments + shares published AT BALLOT time; open
only at close. Exactly our deadline semantics. Optimistic reconstruction
(dealer reveals s) is the cheap path; pessimistic = quorum decrypt.

## ⚠️ THE LANDMINE (third sighting tonight)

**bn254 G1 cannot host the sealed tally.** Every hiding relation in Λ_indv
is DDH-shaped (y_i = PK_i^{f(i)}, U = G^{s+v}, commitments h₀^{f(j)}).
bn254 pairings solve DDH: e(U, h₀)/e(y₀, G) = e(G, h₀)^v → **the sealed bit
is readable by anyone who can call the pairing.** Same class as the FlowX
wrap lesson and the ElGamal-on-bn254 catch: type-system says encrypted,
pairing says glass.

→ The Λ_indv group must be DDH-hard: ristretto255 or secp256k1.
→ NEAR hosts have NO generic-curve ops (only altBn128 + stitched BIP340).

## Two integration architectures

**Arch A — bulletin-board + schnorr quorum (pragmatic, recommended first):**
Chain stores: roster root, transcripts (y⃗, U, π's — opaque bytes), nullifier
dedup, deadline. Verification of π_3PVSS/π_U runs OFF-chain by every member
and any watcher (n=8: milliseconds each, ristretto ops). At close, quorum
members compute V* off-chain and co-sign the result statement via FROST/BIP340
schnorr (repo: `schnorr/`, NEAR host: stitched schnorr verify) → contract
checks quorum-sig + challenge window (optimistic: any member can dispute
within Δ by posting a fraud proof; on-chain adjudication only then).
On-chain crypto: tonight's Groth16 identity proofs + schnorr. NO new hosts.
Trust: on-chain execution gated by quorum honesty + dispute window — matches
the paper's own "optimistic reconstruction" design.

**Arch B — in-circuit verification (stronger on-chain guarantees, heavier):**
Wrap Λ_indV Verify + π_U inside Groth16: prove "transcript verifies AND bit
bound AND (blinded) member identity" in one SNARK per ballot. Requires
ristretto/secp256k1 emulated arithmetic inside the bn254-scalar-field circuit
(Zcash-style ed25519-in-circuit; ~10–30k constraints for our sizes). Kills the
off-chain-verify assumption; cost: circuit engineering + ptau16+ + proving
time per ballot (seconds→tens of seconds). This is the "no watcher needed"
endgame, not the first build.

## Component → repo mapping

| paper component | status |
|---|---|
| member anonymity + nullifier | ✅ deployed tonight (zk-identity, Groth16 @ 33.7 Tgas) |
| deadline + dedup + state contract | ✅ pattern live (squad v0 contract) |
| Λ_indv Share/Verify/Reconstruct | 🆕 ristretto impl (~300 lines, no trusted setup) |
| π_U OR-proof | 🆕 ~60 lines (Fiat-Shamir over Keccak/Poseidon) |
| quorum signatures (Arch A) | 🔨 bip340 in repo; FROST = upgrade |
| fraud-proof adjudication | 🆕 challenge window + dispute fn |
| in-circuit verify (Arch B) | 🆕 research lane, optional |

## Effort

- Arch A skeleton (ristretto Λ_indv + contract + 8-member local run):
  **2–4 focused evenings.**
- Arch A hardened (fraud window, cover traffic, tamper tests): +1 week.
- Arch B: separate lane after A proves the shapes.

## Output modes (design dial, unchanged from v0 spec)

Close opens Σv exactly (margin public), or quorum additionally proves
"Σv ≥ t" via ZK on the reconstruction transcript → world learns ONLY the bit.
