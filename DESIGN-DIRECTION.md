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

**Open (under active research):** whether paid liquidity providers are *empirically* net-profitable
on persistently lopsided books, and whether spot-margin survives thin-memecoin gap risk — using
real data from the protocols that have actually run one-sided flow (Hyperliquid HLP, GMX GLP,
Gains/gTrade, Jupiter JLP, Drift, Lavarage). The recommendation between A and B, with the specific
parameters that make each work or fail, will be appended here when that research lands.

## On the `parimutuel/` folder

The clean-room model there proves the settlement core is **solvent-by-construction and
conserving** (model stress tests + formal proofs). That result stands and underpins the
tail-bounding in Design A. What it does *not* do is make a one-sided market free — see the iron
law. Treat `parimutuel/` as the validated settlement primitive, not the finished product.
