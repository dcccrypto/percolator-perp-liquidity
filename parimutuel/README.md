# Parimutuel settlement (current direction)

Funding long-tail perps **without a vault taking the directional other side.** The vault /
vAMM-as-counterparty model bleeds when flow is one-sided (which long-tail and memecoin flow
usually is), which is a permanent tax on the LP and creator and isn't a real long-term answer.
This swaps the *settlement backbone* for a parimutuel one while keeping the leveraged-perp feel.

## The idea
Think horse betting. Longs and shorts each post collateral into their own pool. Winners are paid
**out of the losing pool**, bounded by what the losing side actually put up. The protocol never
holds a position, so it can't go insolvent — it only redistributes deposited collateral. On top
sits a normal perp UX: leverage, continuous PnL, a liquidation price, no expiry.

## How it maps onto Percolator's engine
Most of it is already there:
- **The payout is already parimutuel-shaped:** `credit_rate = min(1, available_backing / claims)`
  (`v16.rs:406-417`). The change is to point "backing" at the **opposing side's collateral**
  instead of the LP vault.
- **The two sides already fund each other:** each asset splits into a long domain and a short
  domain, and PnL routing already sends a winner's claim to the opposite domain and a loser's
  loss into their own domain's backing (`settle_leg_kf_effects_at_slot`, `v16.rs:7916-7930`).
- **Matched book** keeps `oi_eff_long == oi_eff_short`, so quantity is always two-sided; only
  *collateral* asymmetry (differential leverage) makes `credit_rate < 1`.

So the engine change is mostly re-plumbing where backing comes from. The **matcher (vAMM) change**
is the real new work: match takers **peer-to-peer first**, and bound any residual the house holds
by **real opposing collateral (or a fixed seed), never an unbounded vault.** (Detailed matcher
spec lands in `MATCHER-CHANGES.md`.)

## What a trader experiences
- **Deep, two-sided book:** `credit_rate == 1`, full 1:1 payout — indistinguishable from a normal perp.
- **Thin, one-sided book:** a transparent, on-chain-readable haircut bounded by the opposing pool,
  instead of a vault quietly detonating.
- **Funding** becomes pool-imbalance: it flows from the crowded side to the thin side, paying
  traders to take the unpopular side, so the book self-balances.
- **Leverage** is gated to how deep the other side is, so the cap stays invisible at the offered
  leverage (the binding side is the thin/loser side — a "5x" market must be governed by the
  shorts' depth, not the longs').

## What's in this folder
A self-contained, integer (`u128`) **model** of the settlement math + a stress harness + Kani
proofs. A clean-room model to validate the economics, **not** the production engine.

- `proof_payout_bounded_by_opposing_pool` — winners can never draw more than the losing pool holds (solvent-by-construction).
- `proof_conservation` — total out == total in, every settlement (nothing to bleed, nothing minted).
- `proof_full_payout_when_funded` — a funded book pays 100% (the normal-perp case).
- `proof_credit_rate_threshold` — exactly when the haircut appears (`claim > backing`).
- `proof_loser_limited_liability` — a loser never owes beyond their margin.
- `proof_house_residual_loss_bounded` — with peer-to-peer + a fixed seed, the house's loss on a
  one-sided residual is bounded by the seed, never the vault (the matcher change, formalized).

Every proof is paired with `kani::cover!` non-vacuity witnesses. The stress harness sweeps
solvency + conservation across ~10,500 combinations of volume, leverage, imbalance, and price move.

```bash
cd parimutuel
cargo test     # stress sweep + worked examples (no Kani needed)
cargo kani     # the formal proofs + non-vacuity covers
```

## Honest open items
- The matcher peer-to-peer + bounded-residual change is genuine work (the vAMM is currently the
  vault-backed passive maker). See `MATCHER-CHANGES.md`.
- Engine touches land in both mirrors (zero-copy + runtime-vec), so each must be made twice and re-proven.
- Cold-start: the first matched pair must post both legs' collateral atomically.
- The vault token account is shared per market-group, so an MVP runs one asset per group (or adds
  per-market vault segregation).

This is a design + validated model, not a deployed implementation.
