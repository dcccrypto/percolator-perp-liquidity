# AMM fee-engine validation (spot)

`EdgeMax_CumVar.rs` is the volatility-adaptive constant-product fee curve, empirically
validated on [benedictbrady/prop-amm-challenge](https://github.com/benedictbrady/prop-amm-challenge)
(a spot-AMM simulation that scores "edge" = retail spread captured − LVR paid to arbitrageurs,
1000 sims × 10k steps, against a fixed-fee competitor).

It is the empirical evidence for the tradability / fee half of the design.

## Strategy
Integer constant-product AMM with a cumulative-mean (MLE) variance estimator of the per-step
reserve-implied price move, driving a dynamic fee:

```
fee_bps = clamp( 20 + 0.7·σ̂ + σ̂²/160 , 20 , 130 )
```

The fee depends only on reserves and stored state, never on the input, so each side's quote
stays a pure CPMM-with-fee curve (monotone and concave). All math is integer (`u128`).

## Result
| seed block | avg edge |
|---|---|
| in-sample (0–999) | 423.6 |
| OOS (100000–) | 422.3 |
| OOS (200000–) | 418.5 |

About **+6% over the best fixed fee** (~400 at 65 bps), robust out-of-sample, and
`prop-amm validate` passes with native↔BPF parity delta = 0 (integer-only, so the server
score equals the local score).

## Why it matters here
A vol-adaptive fee beats a fixed-fee competitor on a real, third-party, gradeable engine. It
is the matcher fee logic the perp design layers on.

**Scope (honest):** the challenge is spot, fully-collateralized, and non-adversarial, so this
validates the fee/liquidity engine, not the perp solvency model. The solvency invariants are
formally proven in [`../proofs`](../proofs).

## Run
Clone the challenge repo, drop this file in, then:
```
prop-amm validate EdgeMax_CumVar.rs
prop-amm run      EdgeMax_CumVar.rs
bash eval.sh      EdgeMax_CumVar.rs
```
