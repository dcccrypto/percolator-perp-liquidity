# Permissionless perps on long-tail tokens — design direction

This is the current, honest state of the thinking. It supersedes the "parimutuel settlement"
framing in `parimutuel/`: that work is real and the settlement core is proven solvent, but a
bounded creator seed turned out to be **bad product** for the markets we actually care about, so
the direction has moved on. This doc explains why, and where it's going.

## The goal

Let anyone launch a leveraged market on any token — memecoins and long-tail assets included —
that is **safe** (cannot be drained or made insolvent), gives traders a **real** leveraged
experience, and does not depend on professional market makers or deep pre-existing liquidity.

## The iron law (why there is no free lunch)

In any leveraged market, when one side wins, the money is paid by someone who loses. On a
one-sided book (everyone long a pumping memecoin), the winners' money can only come from one of
exactly four places:

1. **Real opposing traders** — don't exist on a one-sided book, by definition.
2. **A vault / LP** acting as the standing counterparty.
3. **The market creator** (a posted stake).
4. **The token's own spot liquidity** — i.e. the system actually *holds the token*, so it rises
   with the longs.

There is no fifth source. So the real design question was never "how do we avoid a loser." It is
**"who is the counterparty, and are they paid enough to take that side willingly and survive?"**

## What we tried, and why each falls short

1. **Vault as counterparty (the vAMM model).** An automated pool takes the other side of every
   trade. On one-sided memecoin flow it bleeds without bound; a sustained one-way run can drain it
   (this is the Hyperliquid HLP / JELLY failure mode, March 2025). Rejected.

2. **Pure parimutuel (two pools fund each other).** Longs and shorts post into their own pools;
   winners are paid out of the losing pool, capped by it; the protocol holds no position and
   therefore cannot go insolvent. This genuinely fixes **solvency** — but only solvency. On a
   one-sided book there is no losing pool, so it pays the winning side **almost nothing** even
   when they were right. Safe, but not a usable product on its own.

3. **Parimutuel + a bounded creator seed.** Add a small, fixed creator stake as the residual
   counterparty so the crowded side can actually fill. The protocol still can't go insolvent, and
   the creator's loss is hard-capped at the seed. **The problem:** memecoin markets are
   *structurally* one-sided — imbalance is the normal state, not a tail event — so a loss-only
   seed bleeds as a routine occurrence. The creator is forever refilling a draining stake, and
   liquidity dries up once it's gone. Capped, but still bad product.

## The reframe: the counterparty must be *paid*, not just *bounded*

A counterparty that only caps its loss is a bleeder. A counterparty that is **paid** to take the
unpopular side — through funding, trading fees, and the price-impact spread — and is **tail-bounded**
so it can never be wiped, is a *profitable role on average*. The question "does it bleed?" then
stops being structural doom and becomes a **pricing question**: are funding and fees set above the
expected directional cost? This is, in fact, how the surviving perp-DEX liquidity backstops stay
net-positive over time; their only fatal flaw has been an **unbounded tail**, which the
parimutuel-style cap fixes.

## The two candidate designs

**A) Paid, tail-bounded liquidity perp.** Keep the matched-book perp. Match real takers
peer-to-peer first (free, no one bleeds). For the one-sided residual, a liquidity provider takes
the other side and is compensated by funding + fees + spread, with a parimutuel-style cap so its
tail loss is bounded and the protocol can't go insolvent. Funding is cranked per-market so a
lopsided book pays the provider richly *and* lures arbitrage onto the thin side. The provider is a
yield position, not a structural loser. Risk: mispriced funding/fees. Fits the existing engine.

**B) Spot-margin collateralized long.** Drop the synthetic short entirely (source #4 above).
Back a leveraged long with the **real token held as collateral** (deposit → borrow stablecoin →
buy token → the token *is* the collateral, liquidated if it drops). The counterparty is
stablecoin **lenders** who earn interest and never take price risk. When the coin pumps the longs
win because the system actually holds the coin — nobody bleeds. It serves the popular (long) side
perfectly; the short side is naturally limited, which for memecoins is fine because nobody wants
it. Risk moves to **liquidation gap / bad debt** on a fast dump (normal, well-understood lending
risk, managed with conservative LTV and liquidation incentives), not a constant directional bleed.
This is a different architecture (a money-market + spot loop), likely a separate build rather than
the perp engine.

## What's decided vs open

**Decided:** the unbounded vault and the loss-only bounded seed are both out. The counterparty
must either be paid (A) or replaced by the token's own spot (B). Either way, solvency is
non-negotiable and is achievable.

## What the research found (sourced)

A cross-protocol review (Gains/gTrade, Jupiter JLP, GMX v2, Drift, Hyperliquid, Lavarage) of how
real protocols handle one-sided flow:

- **Design A is exactly how every perp-LP backstop already works — and it confirms the bleed.**
  Gains gToken, Jupiter JLP, and GMX GM pools are *by design* the direct counterparty: they pay
  trader profits and absorb losses. They are net-positive **only when accumulated fees exceed net
  trader payouts** — Jupiter's own docs concede that "in a sustained directional rally where most
  traders are correctly positioned, fees may not cover PnL paid out." That sustained one-way rally
  *is* the memecoin case. So a paid LP doesn't escape the bleed; it just bets fees out-run it.
  ([gains.trade](https://docs.gains.trade/liquidity-farming-pools/gtoken-vaults),
  [jup.ag](https://hub.jup.ag/guides/perpetual-exchange/how-it-works),
  [gmx](https://github.com/gmx-io/gmx-synthetics/blob/main/README.md))
- **The thin-side subsidy is finite and degrades; it does not guarantee a counterparty.** Funding
  pays the unpopular side (Hyperliquid even floors shorts at ~11.6% APR), but Drift's Rebate Pool
  *caps* funding receipts when it runs dry and clamps magnitude by tier. So "crank funding to pull
  in shorts" has a hard ceiling — past it, the book stays one-sided.
  ([hyperliquid](https://hyperliquid.gitbook.io/hyperliquid-docs/trading/funding),
  [drift](https://docs.drift.trade/trading/funding-rates))
- **The thing that actually makes permissionless long-tail safe is hard per-market risk
  isolation — not bigger subsidies.** GMX isolates LP P&L per market; Drift's long-tail
  ("Highly Speculative") tier has **no external insurance** — insurance comes only from that
  market's own fees. Plus per-market OI caps, reserve factors, and price ceilings explicitly "to
  reduce the risk that long positions cannot be fully paid out," and large skin-in-the-game gates
  (HIP-3: stake 500k HYPE / ~$25M for 183 days, deployer sets the oracle + caps).
  ([gmx](https://github.com/gmx-io/gmx-synthetics/blob/main/README.md),
  [drift prelaunch](https://docs.drift.trade/trading/prelaunch-markets),
  [HIP-3](https://hyperliquid.gitbook.io/hyperliquid-docs/hyperliquid-improvement-proposals-hips/hip-3-builder-deployed-perpetuals))
- **Design B (spot-margin) is structurally different: the counterparty is senior, not
  directional.** Lavarage is peer-to-peer lending — the trader borrows quote and *holds the real
  spot token* as collateral; lenders are senior to the token price (a loan against collateral),
  not exposed to directional P&L the way a perp LP is. The short side simply doesn't need to
  exist, which fits the long-only memecoin demand.
  ([lavarage](https://lavarage.gitbook.io/lavarage/platform/liquidity))

**Two honest gaps the research could NOT close:** (1) No independently-verified realized LP-vault
P&L survived verification — the widely-cited "HLP was net-profitable" figure was *refuted*, so
nobody has publicly *proven* a profitable permissionless one-sided memecoin leverage product.
(2) The claim that Lavarage lenders are shielded from bad debt was *also refuted* — so Design B's
gap-risk on thin memecoin liquidity is a real, unquantified exposure, not a solved property.

## Recommendation (provisional)

On mechanism (the only thing the evidence actually supports), **Design B is the structurally safer
fit for one-sided memecoin demand**: the counterparty earns interest and never takes the token's
directional risk, whereas Design A puts a provider on the unpopular short side where it
structurally bleeds whenever the crowd is right — which, for memecoins, is the whole thesis. The
catch is that B is a **different product** (a money-market + spot loop, not the matched-book perp
engine), it only serves the *long* side, and its bad-debt/gap risk on thin liquidity is real and
unproven.

So the real fork is strategic, not just technical:
- **Keep the perp engine → Design A**, accepting it is the GMX/HLP model: viable *only* with a
  genuine hard loss cap, strict per-market risk isolation (one market = its own segregated
  insurance), and a hard OI cap sized to the seed — and even then it can degrade and has no
  proven profitability on memecoins.
- **Serve the actual demand best → Design B (spot-margin long)**, accepting it's a pivot away from
  the perp engine to a lending/spot product, long-only, with lending-style bad-debt risk to
  manage via conservative LTV, liquidation buffers tuned to thin DEX depth, and robust oracles.

Either way: **nobody has publicly cracked profitable permissionless one-sided memecoin leverage**,
so this is frontier territory, and the universal safety requirement is per-market isolation + hard
caps + a trustworthy oracle. No design removes the cost of one-sided flow; B relocates it from
"LP bleeds on direction" to "lenders bear gap risk," which is the better-understood, more
bounded risk for the long-only case.

## On the `parimutuel/` folder

The clean-room model there proves the settlement core is **solvent-by-construction and
conserving** (model stress tests + formal proofs). That result stands and underpins the
tail-bounding in Design A. What it does *not* do is make a one-sided market free — see the iron
law. Treat `parimutuel/` as the validated settlement primitive, not the finished product.

## Design A — spec outcome (deep design pass)

A full design + adversarial-critique pass produced a consolidated spec. Headlines:

- **Zero changes to the core settlement engine.** Every property Design A needs (the solvency
  cap, cross-side funding, loss-bounded-by-own-capital, the matched book) already exists and is
  reused as-is. The build is: **one new on-chain "residual-signer" program** (~440 LoC) that lets
  a program-owned seed sign as the residual counterparty, **additive matcher changes** (a
  seed-proportional fill cap + pool state), and **activation-time config** (binding the program as
  the backing authority and enabling funding at market creation) — no edits to the trade engine.
- **Two fill paths:** real takers matched **peer-to-peer** (zero house risk); the one-sided
  residual filled by a **bounded, paid seed** (earns fees + spread + funding). The seed's loss is
  hard-capped by a per-market **OI cap sized to the seed**, so the protocol can never go insolvent.
- **Per-market isolation** (one market group per token, enforced on-chain) so a blown market can't
  touch others.
- **Oracle tiers:** A (Pyth/Switchboard direct) and B (composed) are safe; **Tier C (fresh
  memecoins with only a thin DEX price) is NOT fully safe** — a thin pool can be flash-manipulated
  for less than the seed. Tier C ships behind a flag at dust caps, or not at all.

**The two honest blockers (no surprises):**
1. **Taker routing.** The engine needs *both* trade legs to sign the same atomic transaction and
   has no relayer/permit path. The seed leg is solved (the new program signs it), but the *real
   taker* must co-sign each fill, or a pre-signed-order relay must be built. This is the
   load-bearing unsolved piece and it gates the build scope.
2. **Tier C oracle manipulation** (above) — the marketed memecoin case is the least-safe one.

Plus: it's **permissionless to launch but depends on a trusted keeper to operate** (mitigated by a
permissionless fallback crank, an on-chain funding-sign check, and a no-keeper close path), and
**profitability is unproven at the frontier** — the seed is a tail-bounded *paid* role that
survives by market selection, not guaranteed yield.

The full technical spec (with engine internals) is kept private per the scrub rule.
