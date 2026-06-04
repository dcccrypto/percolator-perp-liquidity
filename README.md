# Permissionless Perps for Any Token

A design for launching a tradable perpetual-futures market on **any** token with a small,
refundable creator stake instead of deep liquidity or paid market makers.

A bonding curve let memecoins launch on the **spot** side with nothing seeded, because the
curve holds the money and can always pay out. This is the equivalent for **perps**.

> **Status: this is a design with validated components and a concrete build path — not a
> deployed system.** The fee engine and the core economic invariants are validated; the
> on-chain vAMM itself is a bounded build, described honestly below. Shared for feedback.

## Contents
1. [The problem](#the-problem)
2. [Who actually provides the liquidity](#who-actually-provides-the-liquidity)
3. [How it works](#how-it-works)
4. [The safety model](#the-safety-model)
5. [What's validated, and what isn't](#whats-validated-and-what-isnt)
6. [Running it on a real engine: what it takes](#running-it-on-a-real-engine-what-it-takes)
7. [The dynamic / lifecycle frontier](#the-dynamic--lifecycle-frontier)
8. [Repo layout](#repo-layout)
9. [Run the proofs](#run-the-proofs)
10. [Open questions](#open-questions)

## The problem
Opening a tradable perp on a brand-new token needs someone on the other side of every
trade. Big venues pay professional market makers or seed deep pools. A small project can't
afford either, and a professional MM will never quote a thin long-tail perp: there's no flow
to earn the spread on and the adverse selection is brutal. So most new tokens never get a
real perp market, and the major venues only ever list a few dozen majors. A perp is harder
than a spot bonding curve because it is leveraged: when a trader wins, real money has to be
there to pay them.

## Who actually provides the liquidity
This is the crux, so it comes first. The design does **not** try to attract market makers —
that part is unsolvable for thin markets. It changes *where the liquidity comes from*:

- **It's a matched book.** For most of the volume, traders are each other's counterparty:
  every long is matched by a short. The only thing that needs external backing is the
  **residual imbalance** between the two sides, which is a small fraction of the notional.
- **The creator backs the residual.** The party that posts the small first-loss buffer is the
  token's own creator, the one actually aligned. A perp market is volume, attention, and a
  reason to hold their token, and a small refundable stake is far cheaper than a
  liquidity-mining program or a market-maker deal.
- **Yield-seekers can add backing.** Anyone who wants the fee share can deposit additional
  backing, and the buffer compounds from trading fees.
- **It stays capped to the buffer.** Open interest is bounded by what the buffer can actually
  cover, so the market is sized to real demand instead of needing depth up front.

So "who provides liquidity" reframes to "who backs the residual skew," and the answer is a
small creator stake, the traders themselves, and optional yield LPs, growing from fees. The
honest open question is demand-side — will creators stake, and is flow two-sided enough that
the buffer isn't just the counterparty to a one-way pump — which is answered by launching
small real markets, not by building more.

## How it works
1. **An automated curve quotes both sides.** The design uses an integer constant-product
   (`x·y=k`) curve recentered on an **endogenous mark** (an EWMA of the market's own trades,
   for tokens with no oracle), with a per-slot price clamp. *Note: a production matched-book
   engine like Percolator currently uses a linear price-impact matcher; the constant-product
   curve is one of the build items in section 6.*
2. **The book is net-flat:** long OI == short OI. The creator-backed vault stands behind the
   residual imbalance, so no external market maker is needed.
3. **A volatility-adaptive fee:** `fee_bps = clamp(a + b·σ̂ + c·σ̂², floor, ceiling)`, where σ̂
   is a running volatility estimate. It reads only from reserves and stored state, never from
   the trade input, so each side stays a monotone, concave curve. All integer (`u128`).
4. **Capacity is hard-capped** at `N_max = β·C_m / R_max`, where `C_m` is the buffer (creator
   stake + locked fees + extra backing), `β < 1` a safety margin, and `R_max` the maximum loss
   per position. Enforced as a reject at trade preflight.
5. **Losses follow a strict waterfall:** loser's margin → creator first-loss stake →
   per-market insurance → proportional auto-deleverage haircut. The creator's stake is
   consumed **before** any other backer's capital.
6. **Winners are paid in full** when backing ≥ claims; only in the extreme tail is the payout
   scaled by a conservation-respecting credit rate `min(1, backing/claims)` — a proportional,
   on-chain-readable haircut.

## The safety model

### The core guarantee
For an isolated market, the protocol and non-creator capital take no loss in normal
operation, and the most that can be extracted is bounded by the market's own capital `C_m`,
with the creator's stake consumed first. Capacity is bounded by `N_max = β·C_m/R_max`.

### Open considerations, and how each resolves
- **Capacity must be capped and the buffer locked.** A matched book bounds net skew but not
  total size, and backing must not be withdrawable while it supports open positions.
  *Resolution:* a hard OI cap at trade preflight, plus backing locked against open interest by
  a proportional floor (you can only withdraw the un-encumbered fraction; at full utilisation,
  none), with the cap tracking live backing so it can't be inflated independently.
- **The creator's stake must absorb losses first.** *Resolution:* a creator-funded first-loss
  layer consumed before any non-creator backer; deleverage only engages once it is empty, and
  then as a proportional haircut. *Residual:* in the deepest tail, once creator stake and
  insurance are both exhausted, the haircut reaches other LPs — by design, and only there.
- **Markets must be isolated.** *Resolution:* per-market segregated insurance (in practice,
  one market per group), so a loss in one market cannot drain another's fund.
- **Liquidation must keep up, or the loss must be bounded when it can't.** *Resolution:* size
  the cap and per-position risk to the worst-case close-out rate so realised loss stays inside
  the buffer (`C_m ≥ OI_cap · P · 2L / k`); cap legs low, keep a tight per-slot clamp, keep OI
  small relative to the buffer. Under extreme congestion the market halts safely rather than
  staying open.
- **The endogenous mark must be hard to move, and not misused externally.** *Resolution
  (internal):* a depth/age gate keeps size capped until the market matures, plus a longer
  smoothing window, a tight clamp, and a protocol-enforced cap, so moving the mark costs more
  than it can yield. *Residual (external):* a third party can still read a public value, so
  external use of a young market's mark is a documented integrator-discipline risk that the
  protocol cannot prevent on-chain.
- **A fully-guaranteed payout, when you want it.** For markets that need every winner paid in
  full with no tail haircut, the maximum payout is escrowed up front and the payoff per
  position is capped. This is the only mode that is fully safe with no caveats; it costs real
  capital.

### Honest residuals
Capital efficiency (the buffer is a few times larger than open interest), the deepest tail
still reaching other LPs unless the pre-funded mode is used, external misuse of a young mark,
and safe-halt rather than continued trading under extreme stress.

## What's validated, and what isn't
- ✅ **Fee curve** — empirically validated against a fixed-fee baseline (~+6% on captured edge,
  out-of-sample, native↔BPF parity). See [`amm/`](amm).
- ✅ **Model invariants** — 7 formal (Kani) proofs + 8 property/worked-example tests of: the
  loss bound (`≤ β·C_m < C_m`), payout conservation (no winner overpaid; all winners ≤
  backing), full payout when funded, curve monotonicity and concavity, capacity growth, and
  OI-cap enforcement — each paired with `kani::cover!` non-vacuity witnesses. See
  [`proofs/`](proofs). (`cargo test` passes; `cargo kani` runs the formal proofs.)
- ✅ **Engine fit** — mapped against a real matched-book engine; the required changes are
  bounded and do not touch the settlement core (section 6).
- ❌ **Not built / not validated** — the on-chain vAMM itself. The proofs verify the *model's*
  invariants, not a deployed engine. The mechanism above is a design.

## Running it on a real engine: what it takes
Mapped against a production matched-book perp engine (Percolator), the primitives mostly
exist — a net-flat book, a proportional credit-rate payout, lien-locked backing, and a safe
halt are native. The safe vAMM needs these changes, **none of which touch the settlement
core**:

1. **Constant-product curve** — the deployed matcher is a linear price-impact model; add a
   CPMM variant recentred on the endogenous mark.
2. **Always-on counterparty** — make the creator/LP vault take the residual leg with a program
   signature, so trades fill without a per-trade human signer. **This is the keystone — its
   feasibility should be prototyped first.**
3. **Per-market capacity cap** — a hard `N_max = β·C_m/R_max` reject at trade preflight (today
   only a single global ceiling is checked, post-write).
4. **Creator-first-loss layer** — consumed before any other backer, **with the deleverage path
   made tranche-aware** (without this, a creator could escape their own first-loss).
5. **Mark hardening** — depth/age gate, a real per-slot clamp, fee floors, and a
   protocol-enforced (not creator-set) cap.
6. **Funding for one-sided exposure**, and a fix so winners cannot be **permanently frozen** in
   the resolution path.

**Honest status:** with deploy + config only, a market mechanically runs and trades, but it is
the *unsafe* shape — no creator-first-loss, no per-market cap, a wide-open mark, a linear
curve, and a counterparty that must co-sign every trade. The full, safe vAMM is the bounded
code project above, plus formal proofs ported into the engine and an audit. It is realistic
(a focused build, settlement core untouched), not a research project — but it is a build, not
a config flag.

## The dynamic / lifecycle frontier
A static cap is only the starting point. The real design question is how a market *breathes*:

- **A cap that breathes** — one capacity multiplier that ramps **up slowly** on hard-to-fake
  signals (real wall-clock age + surviving un-withdrawn buffer + kept fees, never wash volume)
  and **snaps down fast** on stress (buffer drain, vol spike, any haircut), through a single
  debounced trigger so churn can't grief it.
- **Lifecycle phases** — Bootstrap → Growing → Mature → Graduated → WindDown, widening the cap
  and relaxing the mark only as a market proves itself, with an orderly wind-down.

The hardest open problems here: making the deleverage path tranche-aware so first-loss ordering
is genuinely senior-protecting, grounding `R_max` in what the engine actually liquidates at
(not a config-validity bound), keeper incentives for the safety cranks, and per-creator
aggregate exposure when markets are cheap to spin up.

## Repo layout
- [`proofs/`](proofs) — integer reference model of the load-bearing economics + Kani proofs +
  property tests.
- [`amm/`](amm) — the validated volatility-adaptive fee curve and how it was evaluated.
- `README.md` — this document (the full explainer).

## Run the proofs
```bash
cd proofs
cargo test     # property + worked-example tests, incl. non-vacuity guards
cargo kani     # the 7 formal proofs + reachability checks (optional; slower)
```

## Open questions
Design stage, looking for feedback. The sharpest unknowns:
- Is creator-seeded first-loss enough to bootstrap, or does day-one demand need a yield
  incentive for outside backers?
- Is flow on a long-tail token two-sided enough that the buffer isn't just the counterparty to
  a one-way pump?
- The safety items in section 6 — especially the always-on counterparty (keystone) and
  tranche-aware deleverage (the one that's actively unsafe if gotten wrong).
