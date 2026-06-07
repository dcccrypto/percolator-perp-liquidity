//! Parimutuel perp engine — clean fork-model.
//!
//! This is a self-contained, integer (u128) model of the "leveraged-perp feel on top,
//! parimutuel settlement underneath" design, MERGED with what Percolator's engine already
//! has: the payout is the engine's own `credit_rate = min(1, backing/claims)`,
//! and the two sides fund each other through the engine's existing long/short domain split.
//! The one structural change modeled here is that "backing" is the OPPOSING TRADERS' collateral
//! (their margin), NOT an LP vault. The protocol therefore never holds a directional position.
//!
//! MODEL SCOPE — read this note before leaning on any claim here. This is an economic model,
//! NOT a 1:1 mirror of the engine. Deliberate omissions, each verified against the real source:
//!   * The production settlement computes backing from more than one term (it can include a
//!     separate protocol-funded component), so on-chain a winner can be paid MORE than the losing
//!     pool posted. This model uses a single opposing-collateral term, so "bounded by the opposing
//!     pool" is a property of THIS model only and must be re-verified on-chain.
//!   * The production credit_rate denominator is a per-domain AGGREGATE claim bound, not the
//!     single-pair `n*r` used here.
//!   * Liens, the bound-vs-exact split, and backing expiry are not modeled.
//! What IS faithfully proven: a single-step pool-vs-pool transfer clamped by min(available/claim,1)
//! is solvent and conserving, and the residual house loss is bounded by a fixed seed.
//!
//! WHAT THIS PROVES (the design's load-bearing claims), non-vacuously:
//!   * SOLVENT-BY-CONSTRUCTION: winners can never be paid more than the opposing pool holds.
//!   * CONSERVATION: total paid out == total deposited, always. The protocol never adds or
//!     loses a unit — there is nothing to "bleed" (the JELLY failure cannot happen here).
//!   * FULL PERP FEEL when funded: when the opposing pool covers the claim, winners get 100%.
//!   * THE CAP THRESHOLD: exactly when the haircut becomes visible (claim > opposing pool),
//!     which maps to the closed form `r > rho/k` used to size leverage per market.
//!
//! RUN:
//!   cargo test            # the stress harness (knob/volume sweeps) + worked examples
//!   cargo kani            # the formal proofs + non-vacuity cover checks
//!
//! All arithmetic is saturating/integer to match BPF; no floating point.

/// Design-A mechanisms (OI-cap sizing, paid-LP break-even, funding, seed exhaustion,
/// thin-rebate, residual depth clip) — a faithful integer model of the design spec
/// sections 4/6/8, built on the credit_rate/pool_draw primitives below.
pub mod design_a;

/// Basis-points denominator (1.00 == 10_000 bps).
pub const BPS: u128 = 10_000;
/// Credit-rate fixed-point scale (1.0 == SCALE). NOTE: the real engine uses CREDIT_RATE_SCALE =
/// 1e12 (this model uses 1e6); the ratio cancels for the rate magnitude but rounding at the
/// haircut boundary differs. This is a model scale, not the engine's.
pub const SCALE: u128 = 1_000_000;

/// The engine's credit rate: the fraction (in SCALE units) of a winning claim that is payable
/// from `backing`. Capped at 1.0; 1.0 when there is nothing owed. THIS IS THE PARIMUTUEL RULE —
/// here `backing` is the opposing side's collateral, not a vault.
pub fn credit_rate(backing: u128, claim: u128) -> u128 {
    if claim == 0 {
        return SCALE;
    }
    let r = backing.saturating_mul(SCALE) / claim;
    if r > SCALE {
        SCALE
    } else {
        r
    }
}

/// What the winning pool actually draws from the opposing (losing) pool to pay winners:
/// `claim * credit_rate`. By construction this is <= `backing` (solvency) and == `claim` when
/// fully funded (full payout). This is the only money that crosses between the two pools.
pub fn pool_draw(claim: u128, backing: u128) -> u128 {
    claim.saturating_mul(credit_rate(backing, claim)) / SCALE
}

/// Collateral (margin) a side posts for `notional` at integer leverage `k` (k x). Higher
/// leverage = thinner collateral backing the same notional = thinner pool for the other side.
pub fn collateral_for(notional: u128, k: u128) -> u128 {
    if k == 0 {
        return notional;
    }
    notional / k
}

/// Outcome of settling one price move between the two pools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settlement {
    pub winners_out: u128, // winners' margin returned + their share of the opposing pool
    pub losers_out: u128,  // losers' margin minus what was transferred away
    pub pool_draw: u128,   // amount moved from loser pool to winner pool
    pub credit_rate: u128, // the haircut factor applied to winners (SCALE == no haircut)
    pub total_in: u128,    // collateral deposited (both sides)
    pub total_out: u128,   // collateral returned (both sides)
}

/// A matched-book market: long and short carry EQUAL notional `n` (the engine forces
/// oi_eff_long == oi_eff_short), but may post DIFFERENT collateral (different leverage).
#[derive(Clone, Copy, Debug)]
pub struct Market {
    pub n: u128,       // matched notional on each side
    pub m_long: u128,  // long side total collateral (margin)
    pub m_short: u128, // short side total collateral
}

impl Market {
    /// Build from per-side leverage.
    pub fn from_leverage(n: u128, k_long: u128, k_short: u128) -> Self {
        Market {
            n,
            m_long: collateral_for(n, k_long),
            m_short: collateral_for(n, k_short),
        }
    }

    /// Settle a price move of `r_bps` (a fraction r = r_bps/BPS). `up == true` means price up
    /// (longs win, shorts lose). The winners' claim on the opposing pool is the PnL `n * r`;
    /// the winners draw `pool_draw(claim, opposing_collateral)` from the losers, capped by it.
    pub fn settle(&self, up: bool, r_bps: u128) -> Settlement {
        let (m_w, m_l) = if up {
            (self.m_long, self.m_short)
        } else {
            (self.m_short, self.m_long)
        };
        let claim = self.n.saturating_mul(r_bps) / BPS; // winners' PnL = N * r
        let cr = credit_rate(m_l, claim);
        let draw = pool_draw(claim, m_l); // <= m_l, == claim when funded
        let winners_out = m_w.saturating_add(draw);
        let losers_out = m_l - draw; // draw <= m_l, so no underflow
        Settlement {
            winners_out,
            losers_out,
            pool_draw: draw,
            credit_rate: cr,
            total_in: m_w.saturating_add(m_l),
            total_out: winners_out.saturating_add(losers_out),
        }
    }

    /// Does the winner take a visible haircut on this move? (credit_rate < 1)
    pub fn cap_binds(&self, up: bool, r_bps: u128) -> bool {
        self.settle(up, r_bps).credit_rate < SCALE
    }
}

/// Simulate a PERPETUAL path: a sequence of per-step signed returns. Each step redistributes
/// between the pools (continuous parimutuel = the funding/PnL stream), collateral updates, and
/// conservation must hold at every step. Returns the final pools and the running total moved.
/// `steps`: (up, r_bps) per step. Models price drift over time / volume.
pub fn simulate_path(mut m_long: u128, mut m_short: u128, n: u128, steps: &[(bool, u128)]) -> (u128, u128, u128) {
    let total_in = m_long.saturating_add(m_short);
    let mut moved_total = 0u128;
    for &(up, r_bps) in steps {
        let (m_w, m_l) = if up { (m_long, m_short) } else { (m_short, m_long) };
        let claim = n.saturating_mul(r_bps) / BPS;
        let draw = pool_draw(claim, m_l);
        let (nw, nl) = (m_w.saturating_add(draw), m_l - draw);
        if up {
            m_long = nw;
            m_short = nl;
        } else {
            m_short = nw;
            m_long = nl;
        }
        moved_total = moved_total.saturating_add(draw);
        // INVARIANT every step: nothing created or destroyed.
        debug_assert_eq!(m_long.saturating_add(m_short), total_in);
    }
    (m_long, m_short, moved_total)
}

/// Funding as pool imbalance: rate flows from the crowded pool to the thin pool, pulling the
/// book back toward balance. `kappa_bps` is the funding aggressiveness. Returns signed-ish
/// magnitude as (toward_short: bool, rate_bps). When pools are balanced, rate == 0.
pub fn funding_rate_bps(m_long: u128, m_short: u128, kappa_bps: u128) -> (bool, u128) {
    let tot = m_long.saturating_add(m_short);
    if tot == 0 || m_long == m_short {
        return (false, 0); // balanced (or empty) -> no funding, direction irrelevant
    }
    if m_long > m_short {
        // longs crowded -> longs pay shorts -> attract shorts
        (true, kappa_bps.saturating_mul(m_long - m_short) / tot)
    } else {
        (false, kappa_bps.saturating_mul(m_short - m_long) / tot)
    }
}

/// MATCHER CHANGE (vAMM design update): PEER-TO-PEER first. Given long flow and short flow, the
/// matched part is min(long, short) — those traders are each other's counterparty with ZERO
/// house exposure. Only the residual |long - short| is one-sided and needs handling.
pub fn matched_residual(long_flow: u128, short_flow: u128) -> (u128, u128) {
    let matched = if long_flow < short_flow { long_flow } else { short_flow };
    let residual = if long_flow > short_flow { long_flow - short_flow } else { short_flow - long_flow };
    (matched, residual)
}

/// The house/creator no longer takes UNBOUNDED directional risk (the old vAMM-as-vault role).
/// If it backs the residual at all, it does so with a FIXED seed `seed_k`, and its loss can
/// NEVER exceed that seed — beyond it the credit_rate haircut takes over. This is the core of
/// the vAMM counterparty change: a bounded seed, not the whole vault.
pub fn house_loss_on_residual(residual_notional: u128, r_bps: u128, seed_k: u128) -> u128 {
    let exposure = residual_notional.saturating_mul(r_bps) / BPS;
    if exposure < seed_k {
        exposure
    } else {
        seed_k
    }
}

// =============================================================================
// FORMAL PROOFS (Kani). Each asserts an invariant AND covers the non-vacuous states.
// =============================================================================
#[cfg(kani)]
mod proofs {
    use super::*;

    /// PROOF 1 — SOLVENT BY CONSTRUCTION: the winners can NEVER draw more than the opposing
    /// pool holds. This is the core "nothing ever bleeds" property — there is no vault term.
    #[kani::proof]
    fn proof_payout_bounded_by_opposing_pool() {
        let n: u128 = kani::any();
        kani::assume(n >= 1 && n <= 1_000_000_000);
        let m_l: u128 = kani::any();
        kani::assume(m_l <= 1_000_000_000);
        let r_bps: u128 = kani::any();
        kani::assume(r_bps <= 100_000); // up to 1000% move
        let claim = n.saturating_mul(r_bps) / BPS;

        let draw = pool_draw(claim, m_l);
        assert!(draw <= m_l); // THEOREM: cannot pay out more than losers posted

        kani::cover!(draw > 0 && draw == m_l); // non-vacuous: the cap actually binds sometimes
        kani::cover!(draw > 0 && draw < m_l); // and the partial-fill regime is reachable
    }

    /// PROOF 2 — CONSERVATION / NO PROTOCOL PnL: total paid out == total deposited, exactly.
    /// The protocol never gains or loses a unit; there is no money to bleed and none to mint.
    #[kani::proof]
    fn proof_conservation() {
        let n: u128 = kani::any();
        kani::assume(n >= 1 && n <= 1_000_000_000);
        let m_long: u128 = kani::any();
        kani::assume(m_long <= 1_000_000_000);
        let m_short: u128 = kani::any();
        kani::assume(m_short <= 1_000_000_000);
        let up: bool = kani::any();
        let r_bps: u128 = kani::any();
        kani::assume(r_bps <= 100_000);

        let mk = Market { n, m_long, m_short };
        let s = mk.settle(up, r_bps);
        assert!(s.total_out == s.total_in); // THEOREM: exact conservation, every settlement

        kani::cover!(s.pool_draw > 0); // non-vacuous: real value actually moved between pools
    }

    /// PROOF 3 — FULL PERP FEEL WHEN FUNDED: when the opposing pool covers the claim, the
    /// winner is paid 100% (credit_rate == 1, draw == claim). Deep market == normal perp.
    #[kani::proof]
    fn proof_full_payout_when_funded() {
        let claim: u128 = kani::any();
        kani::assume(claim >= 1 && claim <= 1_000_000_000);
        let backing: u128 = kani::any();
        kani::assume(backing >= claim && backing <= 4_000_000_000);

        assert!(credit_rate(backing, claim) == SCALE);
        assert!(pool_draw(claim, backing) == claim); // exact full payout

        kani::cover!(claim > 0);
    }

    /// PROOF 4 — credit_rate is a valid fraction, and is < 1 EXACTLY when underfunded
    /// (claim > backing). This is the cap-bind threshold the leverage-gating rule is built on.
    #[kani::proof]
    fn proof_credit_rate_threshold() {
        let backing: u128 = kani::any();
        kani::assume(backing <= 1_000_000_000);
        let claim: u128 = kani::any();
        kani::assume(claim >= 1 && claim <= 1_000_000_000);

        let cr = credit_rate(backing, claim);
        assert!(cr <= SCALE); // valid fraction
        if backing >= claim {
            assert!(cr == SCALE); // funded -> no haircut
        } else {
            assert!(cr < SCALE); // underfunded -> visible haircut (the cap binds)
        }
        kani::cover!(cr < SCALE);
        kani::cover!(cr == SCALE);
    }

    /// PROOF 5 — limited liability: a loser never loses more than the collateral they posted.
    #[kani::proof]
    fn proof_loser_limited_liability() {
        let n: u128 = kani::any();
        kani::assume(n >= 1 && n <= 1_000_000_000);
        let m_long: u128 = kani::any();
        kani::assume(m_long <= 1_000_000_000);
        let m_short: u128 = kani::any();
        kani::assume(m_short <= 1_000_000_000);
        let up: bool = kani::any();
        let r_bps: u128 = kani::any();
        kani::assume(r_bps <= 100_000);

        let mk = Market { n, m_long, m_short };
        let s = mk.settle(up, r_bps);
        assert!(s.losers_out <= if up { m_short } else { m_long }); // never owes beyond margin
        kani::cover!(s.losers_out == 0); // non-vacuous: full liquidation is reachable
    }

    /// PROOF 6 — THE vAMM CHANGE IS SAFE: with peer-to-peer matching + a bounded seed, the
    /// house's loss on a one-sided residual is ALWAYS <= the fixed seed, never the whole vault.
    /// Formal statement that the new matcher takes no unbounded directional risk.
    #[kani::proof]
    fn proof_house_residual_loss_bounded() {
        let residual: u128 = kani::any();
        kani::assume(residual <= 1_000_000_000);
        let r_bps: u128 = kani::any();
        kani::assume(r_bps <= 100_000);
        let seed_k: u128 = kani::any();
        kani::assume(seed_k <= 1_000_000_000);

        let loss = house_loss_on_residual(residual, r_bps, seed_k);
        assert!(loss <= seed_k); // THEOREM: house exposure is bounded by the seed, not the vault

        kani::cover!(loss == seed_k && seed_k > 0); // non-vacuous: the seed cap actually binds
        kani::cover!(loss > 0 && loss < seed_k); // and the under-seed regime is reachable
    }
}

// =============================================================================
// STRESS HARNESS (cargo test). Knob/volume sweeps; every combination must stay solvent and
// conserve. Also pins the cap-bind threshold to the closed form and demonstrates non-vacuity.
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;

    /// THE BIG SWEEP: across volumes, leverages, imbalance ratios, and price moves, solvency
    /// (draw <= opposing pool) and conservation (out == in) must ALWAYS hold. ~tens of
    /// thousands of combinations. This is the "stress with different knobs / volumes" check.
    #[test]
    fn stress_solvency_and_conservation_hold_everywhere() {
        let volumes = [1_000u128, 100_000, 10_000_000, 1_000_000_000, 250_000_000];
        let levs = [1u128, 2, 3, 5, 8, 10, 15, 20, 50];
        let moves = [0u128, 25, 50, 100, 200, 500, 1_000, 2_000, 5_000, 10_000, 25_000, 50_000, 100_000];
        let (mut saw_full, mut saw_haircut, mut saw_wipe) = (false, false, false);
        let mut combos = 0u64;
        for &n in &volumes {
            for &kl in &levs {
                for &ks in &levs {
                    let mk = Market::from_leverage(n, kl, ks);
                    for &up in &[true, false] {
                        for &r in &moves {
                            let s = mk.settle(up, r);
                            // SOLVENCY: never pay out more than the losing pool held.
                            let m_l = if up { mk.m_short } else { mk.m_long };
                            assert!(s.pool_draw <= m_l, "BLEED n={n} kl={kl} ks={ks} up={up} r={r}");
                            // CONSERVATION: nothing created or destroyed.
                            assert_eq!(s.total_out, s.total_in, "CONSERVATION n={n} kl={kl} ks={ks} up={up} r={r}");
                            // credit_rate is a valid fraction.
                            assert!(s.credit_rate <= SCALE);
                            if s.credit_rate == SCALE && r > 0 {
                                saw_full = true;
                            }
                            if s.credit_rate < SCALE {
                                saw_haircut = true;
                            }
                            if s.losers_out == 0 && m_l > 0 {
                                saw_wipe = true;
                            }
                            combos += 1;
                        }
                    }
                }
            }
        }
        assert!(combos > 10_000, "sweep too small: {combos}");
        // NON-VACUITY: every interesting regime was actually exercised.
        assert!(saw_full, "VACUOUS: never saw a full (uncapped) payout");
        assert!(saw_haircut, "VACUOUS: never saw a haircut (cap never bound)");
        assert!(saw_wipe, "VACUOUS: never saw a full liquidation");
    }

    /// THE CAP-BIND THRESHOLD matches the closed form `r > rho/k` (with rho = opposing/dominant
    /// collateral, k = winner leverage). This is the number the leverage-gating rule uses.
    #[test]
    fn cap_bind_matches_rho_over_k_formula() {
        let n = 1_000_000u128;
        // winner side (longs) at leverage k; loser side (shorts) at leverage k_s.
        for &k in &[2u128, 5, 10, 20] {
            for &k_s in &[2u128, 5, 10, 20] {
                let mk = Market::from_leverage(n, k, k_s);
                // closed form: cap binds when claim > m_short, i.e. n*r/BPS > n/k_s -> r > BPS/k_s.
                let threshold_bps = BPS / k_s;
                for &r in &[threshold_bps.saturating_sub(1), threshold_bps + 1] {
                    if r == 0 {
                        continue;
                    }
                    let binds = mk.cap_binds(true, r);
                    let expected = r > threshold_bps; // > because at r==threshold, claim==m_short -> funded
                    assert_eq!(
                        binds, expected,
                        "threshold mismatch k={k} k_s={k_s} r={r} thr={threshold_bps}"
                    );
                }
            }
        }
    }

    /// THE ASYMMETRIC-LEVERAGE TRAP: a conservative 5x long is still haircut when the shorts
    /// run 20x, because the SHORTS' (opposing) collateral is what runs out. Confirms the
    /// binding side is the thin/loser side, not the side you advertise.
    #[test]
    fn asymmetric_leverage_trap() {
        let n = 1_000_000u128;
        let mk = Market::from_leverage(n, 5, 20); // longs 5x, shorts 20x
        // shorts post n/20 = 50_000; longs win; claim exceeds 50_000 once r > BPS/20 = 5%.
        let s_low = mk.settle(true, 400); // +4%, claim = 40_000 < 50_000 -> funded
        assert_eq!(s_low.credit_rate, SCALE, "4% should be fully funded");
        let s_hi = mk.settle(true, 800); // +8%, claim = 80_000 > 50_000 -> haircut at "5x"
        assert!(s_hi.credit_rate < SCALE, "an 8% move haircuts the 5x long because shorts are 20x");
        assert!(s_hi.pool_draw <= mk.m_short);
    }

    /// PERP FEEL: a deep, balanced book (equal leverage) pays 1:1 across normal moves — no
    /// observable haircut until an extreme move. This is what makes it feel like a normal perp.
    #[test]
    fn deep_balanced_book_feels_like_a_normal_perp() {
        let mk = Market::from_leverage(10_000_000, 10, 10); // 10x both sides, balanced
        for &r in &[100u128, 300, 500, 900] {
            // up to +9%
            assert_eq!(mk.settle(true, r).credit_rate, SCALE, "haircut at r={r} in a deep book!");
        }
        // only an extreme move (> 1/k = 10%) starts to bind:
        assert!(mk.settle(true, 1_100).credit_rate < SCALE);
    }

    /// PERPETUAL PATH: continuous redistribution over a multi-step price path conserves
    /// collateral at every step (the funding/PnL stream). Stresses volume/time.
    #[test]
    fn perpetual_path_conserves() {
        let n = 1_000_000u128;
        let (m_long, m_short) = (collateral_for(n, 10), collateral_for(n, 10));
        let total_in = m_long + m_short;
        // a choppy path: up, up, down, up, down, down...
        let path = [(true, 300u128), (true, 200), (false, 400), (true, 100), (false, 600), (false, 150)];
        let (fl, fs, _moved) = simulate_path(m_long, m_short, n, &path);
        assert_eq!(fl + fs, total_in, "path must conserve collateral end to end");
        assert!(fl <= total_in && fs <= total_in);
    }

    /// FUNDING SELF-BALANCES: imbalance funding always flows from the crowded side to the thin
    /// side (the mechanism that pulls rho back toward 1), and is zero when balanced.
    #[test]
    fn funding_pulls_toward_balance() {
        assert_eq!(funding_rate_bps(500, 500, 10_000), (false, 0)); // balanced -> no funding
        let (toward_short, rate) = funding_rate_bps(900, 100, 10_000); // longs crowded
        assert!(toward_short && rate > 0, "longs crowded -> pay shorts to attract them");
        let (toward_short2, rate2) = funding_rate_bps(100, 900, 10_000); // shorts crowded
        assert!(!toward_short2 && rate2 > 0, "shorts crowded -> pay longs");
    }

    /// WORKED EXAMPLE: 100 longs vs 10 shorts at 10x, coin 2x's. The longs can only split the
    /// shorts' collateral — the protocol pays exactly what the shorts posted, never a cent more.
    #[test]
    fn worked_example_one_sided_moonshot_cannot_bleed() {
        // 100 longs * $100 = $10,000 long notional-collateral basis; model as notional with 10x.
        // Use matched notional n; longs deep, shorts thin (rho small).
        let n = 1_000_000u128;
        let mk = Market::from_leverage(n, 10, 10); // both 10x, but we thin the short pool:
        let mk = Market { m_short: mk.m_short / 5, ..mk }; // shorts posted 1/5 as much (thin)
        let s = mk.settle(true, 10_000); // +100% (a 2x)
        assert!(s.pool_draw <= mk.m_short, "longs can never take more than shorts posted");
        assert_eq!(s.total_out, s.total_in, "and the protocol's books still balance to the cent");
        assert!(s.credit_rate < SCALE, "longs are visibly haircut (thin short pool) — the honest cap");
    }

    /// MATCHER CHANGE: peer-to-peer first means the matched portion has ZERO house exposure;
    /// only the one-sided residual can touch the bounded seed, and the seed caps the loss.
    #[test]
    fn matcher_peer_to_peer_then_bounded_residual() {
        // 100 long flow vs 60 short flow: 60 matched peer-to-peer (no house risk), 40 residual.
        assert_eq!(matched_residual(100, 60), (60, 40));
        // balanced flow -> zero residual -> zero house exposure:
        assert_eq!(matched_residual(50, 50), (50, 0));
        // the house only ever risks a FIXED seed on the residual, never more:
        let residual_notional = 1_000_000u128;
        let seed_k = 50_000u128;
        for &r in &[0u128, 100, 1_000, 10_000, 100_000] {
            assert!(house_loss_on_residual(residual_notional, r, seed_k) <= seed_k, "house loss bounded by seed, r={r}");
        }
        // a small move stays under the seed; a big move pins it exactly at the seed (not the vault):
        assert!(house_loss_on_residual(residual_notional, 100, seed_k) < seed_k);
        assert_eq!(house_loss_on_residual(residual_notional, 100_000, seed_k), seed_k);
    }
}
