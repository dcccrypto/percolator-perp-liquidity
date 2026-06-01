# Safety model

This document states what the design protects, the open considerations that have to be
handled for those guarantees to hold, and how each is resolved. It is written to be honest
about current state: the pricing and fee engine work today, and the safety properties below
are concrete, scoped engine changes.

## The core guarantee
For an isolated market, the protocol and non-creator capital take no loss in normal
operation, and the most that can be extracted is bounded by the market's own capital, with
the creator's stake consumed first. Capacity is bounded by:

```
open interest  ≤  N_max  =  β · C_m / R_max
```

where `C_m` is the market's capital (creator stake + locked fees + extra backing), `β < 1`
is a safety margin, and `R_max` is the maximum loss per position.

## Open considerations, and how they're resolved

### 1. Capacity has to be capped, and the buffer locked
A matched book bounds net skew but not total size, and backing must not be withdrawable
while it is supporting open positions. **Resolution:** enforce a hard open-interest cap at
trade preflight, and lock backing against open interest with a proportional floor — you can
only withdraw the un-encumbered fraction, and at full utilisation, none. The cap must track
the live backing so it cannot be inflated independently of the capital behind it.

### 2. The creator's stake must absorb losses first
**Resolution:** a creator-funded first-loss layer is consumed before any non-creator
liquidity provider is touched. Auto-deleveraging only engages once that layer is empty, and
then as a proportional, visible haircut. **Residual:** in the deepest tail, once the creator
stake and the market's insurance are both exhausted, the haircut does reach LPs — by design,
and only there.

### 3. Markets must be isolated from each other
**Resolution:** per-market segregated insurance, held in a per-market vault, so a loss in
one market cannot reduce another market's fund.

### 4. Liquidation has to keep up, or the loss has to be bounded when it can't
Closing positions is rate-limited, so a fast move can outrun it. **Resolution:** size the
open-interest cap and per-position risk to the worst-case close-out rate, so the realised
loss stays inside the buffer even when close-out lags:

```
C_m  ≥  OI_cap · P · 2L / k
```

(`P` = per-slot price clamp, `L` = max legs per account, `k` = fraction of slots a keeper
lands a transaction). In practice: cap legs low, use a tight per-slot clamp, keep open
interest small relative to the buffer, and run the keeper in parallel. Under extreme
congestion the market halts safely rather than staying open. The cost of this safety is
capital efficiency — the buffer is a few times larger than open interest.

### 5. The endogenous mark must be hard to move, and not misused externally
A young, thin market's price is cheap to move. **Resolution (internal):** a depth gate keeps
size capped until the market clears a minimum age and volume, a longer smoothing window and
tight clamp slow the mark, and a protocol-enforced cap bounds the prize, so moving the mark
costs more than it can yield. **Resolution (external):** the endogenous mark is not exposed
as an oracle field and is flagged as endogenous, so other protocols do not consume it by
accident. **Residual:** a determined third party can still read a public value, so external
use of a young market's mark is a documented integrator-discipline risk, not something the
protocol can prevent.

### 6. A fully-guaranteed payout, when you want it
For markets that need every winner paid in full with no tail haircut, the maximum payout is
escrowed up front and the payoff per position is capped. This is the only mode that is fully
safe today; it costs real capital and caps a single win.

## Where this lands
The fee and pricing engine are sound today. The properties above are a concrete, bounded
amount of engine work — a few hundred lines, a handful of new per-market fields, a
migration, and a set of formal-verification harnesses — and they do not change the core
settlement math. Until they are built and audited, the fully-safe configuration is the
pre-funded, capped-payout mode at small size. Feedback on any of this is very welcome.
