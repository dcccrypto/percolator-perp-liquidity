# Formal proofs — safety invariants

A self-contained, integer (`u128`) **reference model** of the design's load-bearing
economics, with [Kani](https://model-checking.github.io/kani/) formal proofs. Every function
mirrors the on-chain fixed-point semantics, and every proof is paired with `kani::cover!`
**non-vacuity witnesses**, so no invariant is true merely because its precondition is
impossible.

This crate proves the *mechanism is sound*. It is a model, not the deployed engine; the
engine changes that enforce these same properties are described in [`../SAFETY.md`](../SAFETY.md)
and [`../docs/Perp-Liquidity-Technical-Design.pdf`](../docs).

## The proofs

| # | Harness | Property proven | Non-vacuity witness |
|---|---|---|---|
| 1 | `proof_loss_bound` | capped loss `≤ β·C_m < C_m` for any position within the OI cap and any clamped adverse move | bad debt is actually positive and approaches the bound |
| 2 | `proof_payout_conservation` | a winner is never overpaid; all winners together `≤ backing` | a real haircut, a full payout, and an under-payment are all reachable |
| 3 | `proof_full_payout_when_funded` | when `backing ≥ claims`, the winner gets 100% | a positive winning claim is paid in full |
| 4 | `proof_curve_monotone` | larger trade ⇒ never less output | the curve strictly increases somewhere |
| 5 | `proof_curve_concave` | marginal output is non-increasing (diminishing returns), within rounding tolerance | universally quantified |
| 6 | `proof_capacity_growth` | adding capital never shrinks the cap, and `β·C_m < C_m` as `C_m` grows | the market actually grows when capital is added |
| 7 | `proof_oi_cap_enforcement` | accepted opens never exceed the cap; rejections are exactly the over-cap opens | both accept and reject branches are reachable |

The central result is **#1 + #2 + #3** together: winners are paid in full when the vault is
funded; in the tail the shortfall is a proportional, conservation-respecting haircut bounded
by `β·C_m`; the pool can never pay out more than it holds or lose more than the first-loss
capital.

## Run
```bash
cargo test     # property sweeps + worked examples (no Kani needed); these also assert the
               # interesting regimes are exercised (non-vacuity)
cargo kani     # all 7 formal proofs + the kani::cover! reachability checks
cargo kani --harness proof_loss_bound   # a single proof
```

`cargo test` runs in under a second. `cargo kani` is slower (bit-precise model checking; the
two curve proofs use small bounds to stay tractable — the properties are structural).
