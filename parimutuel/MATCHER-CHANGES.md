# Matcher (vAMM) changes for parimutuel — and the honest open problems

Grounded in `percolator-match/src/vamm.rs` and the CPI wiring in `percolator-prog/src/v16_program.rs`.
This documents what changes in the matcher to go from "vault-backed passive maker" to
"peer-to-peer + bounded residual" — and, just as importantly, the real unsolved problems the
deep review surfaced.

## The matcher has two layers
- **Pricing layer** (`compute_passive_execution` `vamm.rs:580-634`, `compute_vamm_execution`
  `vamm.rs:636-711`): pure functions, `exec_price = oracle·(1 ± total_bps)`, no counterparty
  identity. Largely reusable (with small call-site tweaks, see below).
- **Counterparty / inventory layer** (`inventory_base` `vamm.rs:97`, `check_inventory_limit`
  `vamm.rs:713-763`, and the CPI wiring): this is the part parimutuel changes.

Today, every fill is booked **100% against the LP vault** (`handle_trade_cpi` sets `account_b` =
LP vault portfolio, signed by the `matcher_delegate` PDA; `handle_trade_nocpi_zero_copy` books
`taker +size`, `vault −size`). No peer-to-peer matching happens. `max_inventory_abs == 0` in
production = the matcher accepts any size; bounding is delegated to the vault's backing bucket
(`add_fresh_counterparty_backing`, which has no ceiling — the unbounded-vault mechanism).

## What stays (reused)
The pricing math (`compute_*_execution`), the skew-spread formula (`compute_skew_extra_bps`),
the insurance-fee accrual, the `MatcherReturn` ABI (stays v3), the CPI mechanics, the engine call
`handle_trade_nocpi_zero_copy`, the `MatcherCtx` layout below offset 144, and the existing
BackingBucket settlement infrastructure.

## What changes
1. **`MatcherCtx` gains `long_pool_atoms` / `short_pool_atoms`** (carved from the 88-byte
   `_reserved`), `MATCHER_VERSION` → 5, new `MatcherKind::Parimutuel`.
2. **`check_inventory_limit` rewritten**: instead of `max_inventory_abs`, clip the fill to the
   **opposing pool's depth** — `(opposing_pool_atoms + creator_seed_atoms)·1e6 / oracle_price_e6`.
   The house never takes more than real opposing collateral + a fixed creator seed.
3. **`inventory_base` reinterpreted** as pool imbalance → drives the skew spread.
4. **Two fill paths**: (A) peer-to-peer — `account_b` is a real opposing taker; (B) house
   residual — `account_b` is a bounded creator-seed portfolio.
5. **Matcher-delegate PDA seed change** (drop `account_b` from the seeds, since `account_b` now
   varies per fill) — a migration for every deployed market.
6. **New instructions**: `InitParimutuelMarket` (atomically seeds both pools), pool deposit/
   withdraw, and a resting-order surface (`RegisterRestingOrder` / `FillRestingOrder`) for P2P.

**Scope:** ~650–750 lines across the matcher + wrapper + tests (not the "small tweak" the early
read suggested). Delivery order: ctx fields + `MatcherKind::Parimutuel` + `check_inventory_limit`
+ Kani proofs → pool deposit/withdraw → wrapper `handle_trade_cpi` (dual-seed version gate) →
`InitParimutuelMarket` → resting-order P2P infra.

## The honest open problems (do not gloss these)
The review flagged real issues. They don't kill the design, but they're the actual work and they
change the story:

1. **The residual leg has no working signer story (biggest gap).** The engine requires
   `account_b.owner == signer_b.key` AND `expect_signer(signer_b)` (`v16_program.rs:7519/7595`).
   For an automated keeper residual fill, the creator isn't there to co-sign. The only
   program-derived signer is the `matcher_delegate` PDA — which means the **creator-seed portfolio
   must be owned by that PDA**, which isn't designed yet. This must be wired before Path B works.

2. **A one-sided market gives ZERO fills (the core trade-off).** The engine is strictly bilateral:
   every `+size` needs a real `−size` `account_b`. With no vault as the always-available maker,
   when the opposing pool (and the creator seed) is empty, the matcher returns `exec_size = 0`. So
   on the crowded side of a one-sided book, you simply **can't open a position.** This is exactly
   the long-tail/memecoin case the whole effort targets. **Parimutuel trades insolvency-risk for
   market-completeness:** it can never bleed, but a one-sided market can stop filling once the
   creator seed is exhausted. The seed defers this; it doesn't remove it.

3. **The skew fee can only widen the crowded side, never cheapen the thin side.**
   `compute_skew_extra_bps` adds spread on the worsening side and returns 0 (base) on the other —
   there's no below-base narrowing. So the *attractor* of the thin side has to be **funding** (the
   pot-to-pot transfer that pays the thin side), not the skew fee. The skew fee only discourages
   piling on.

4. **Settlement mapping is load-bearing and not yet wired.** Turning `long/short_pool_atoms` into
   the engine's `SourceCreditState` invariants (`provider_receivable_num`, `consumed_liened_backing_num`,
   …) isn't a trivial "pool authority deposits to the bucket" — the shape checks in
   `add_fresh_counterparty_backing` have to be satisfied. Needs a real wiring sketch.

5. **`limit_price` reverts, not zero-fills.** With oracle-dependent fill caps and widened spreads,
   more fills breach the `limit_price` band (`v16_program.rs:7640-7649`) and **revert**, so the
   "succeeds with a zero fill" UX isn't the whole story.

6. **Delegate-seed migration** for all deployed markets is an operational change, best done as a
   version-gated dual derivation rather than a state migration.

## Net
The matcher change is **real and moderate-to-large** (~650–750 LoC), the pricing is mostly
reusable, and the design is solvent-by-construction. The honest cost, surfaced here, is
**market-completeness on one-sided books** (#2) and an **unsolved residual-signer wiring** (#1) —
both of which land squarely in the long-tail case, so they're the things to design next, not
afterthoughts.
