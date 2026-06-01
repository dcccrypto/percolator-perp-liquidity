# Permissionless Perps for Any Token

Launch a tradable perpetual-futures market on any token with a small, refundable creator
stake instead of deep liquidity or paid market makers. An automated curve is the
counterparty from the first block, a creator first-loss buffer is the real money behind
the trades, and the market grows from its own fees.

This repo is an MVP and a design shared for feedback.

## The idea
A bonding curve let memecoins launch on the spot side because the curve holds the money
and can always pay out, so a token trades with nothing seeded. A perp is harder, because
it is leveraged and someone has to be good for the winning side. Here, an automated curve
takes that side, backed by a small creator stake rather than a deep pool. Open interest is
capped to what the buffer can cover, so the market cannot go insolvent. Winners are paid in
full in normal conditions, with a rare, proportional, on-chain-visible haircut only in the
extreme tail.

## What's here
| Path | Contents |
|---|---|
| [`docs/Perp-Liquidity-Design.pdf`](docs) | Plain-English design overview (11 pages). Start here. |
| [`docs/Perp-Liquidity-Technical-Design.pdf`](docs) | Technical design (14 pages): mechanism, math, diagrams, and each open safety item as problem → fix → residual. |
| [`SAFETY.md`](SAFETY.md) | The safety model: what's protected, the open considerations, and how each is resolved. |
| [`proofs/`](proofs) | An integer reference model of the mechanism, with property tests and formal-verification harnesses. |
| [`amm/`](amm) | The volatility-adaptive fee curve, with a note on how it's evaluated. |

## Run the proofs
```bash
cd proofs
cargo test     # property tests + worked examples
cargo kani     # formal-verification harnesses (optional; slow)
```

## Status
This is an MVP and a design for discussion. The pricing and fee engine work today. The
capacity, settlement-ordering, and isolation properties that make it fully safe are
specified in [`SAFETY.md`](SAFETY.md) as concrete, scoped engine changes. Feedback welcome.
