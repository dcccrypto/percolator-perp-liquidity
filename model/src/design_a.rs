//! Design A — Permissionless Perp Launchpad: integer model of the NEW mechanisms.
//!
//! This module extends the clean fork-model in `super` (credit_rate / pool_draw / Market,
//! SCALE = 1e6, saturating-u128) with the Design-A-specific machinery described in
//! `the design spec (this repo)` (sections 4, 6, 8). It is a FAITHFUL INTEGER
//! MODEL, not a 1:1 mirror of the on-chain engine. Everything here is saturating/integer
//! (no float; integer isqrt), to match BPF semantics.
//!
//! WHAT MAPS FAITHFULLY (and is proven non-vacuously below):
//!   * OI cap sized to seed = tighter-of(1:1 seed bound, oracle-gate) — section 6, exact form.
//!     The cap is LOAD-BEARING: it keeps the RAW (un-haircut) crowd claim within the seed so the
//!     winner is paid in full in-tier (proof_oi_cap_raw_claim_bounded / proof_oi_cap_full_payout).
//!     The weaker "realized loss <= seed for ANY move" follows from pool_draw clamping and is
//!     stated honestly as a SEPARATE fact (proof_seed_realized_loss_clamped).
//!   * Paid-LP break-even: income (fee+spread+funding*tau) vs adverse (sigma*sqrt(tau)*0.8) —
//!     section 8. Funding income carries FULL PRECISION (rate*tau before the e9 divide) so
//!     spec-band rates (clamp(100,3000) e9/slot) do not truncate to 0; both regimes (bleed on a
//!     6h fast pump at base-rate funding / earn on a 7-day run at ceiling funding) are reproduced
//!     with the closed form (and 0.8 coefficient) pinned EXACTLY.
//!   * Leverage ratchet initial_margin_bps = clamp(max(1000, 1e4*OI/N_cap), .., BPS) — section 8:
//!     monotone in crowding, 10x empty -> 1x at cap; the gate rejects under-margined late entrants.
//!   * Funding schedule clamp(base + slope*imbalance, 0, max) — section 8: monotone in imbalance,
//!     floored/capped, sign always accepted by funding_transfer.
//!   * Zero-sum funding whose SIGN opposes the imbalance (crowded pays thin); MAGNITUDE is the
//!     payer-capped schedule min(payer*rate/E9, payer) (the cap is exercised at rate>E9) — sec 5/8.
//!   * Fee routing 100%-to-seed: split_insurance_fee conserves and routes all to seed when
//!     the insurance-fee share set to 0 — section 4 (fee routing).
//!   * Seed exhaustion: credit_rate monotonically falls toward 0; out == claim*cr/SCALE exactly
//!     (winner never overpaid, never insolvent); a MID-POSITION 0<cr<1 partial haircut is shown.
//!   * Lockstep close: both legs reduce by the same notional (OI parity),
//!     conservation holds, the loser leg never overpays — section 5 (arithmetic half only).
//!   * Seed account (posted vs realized_loss vs at_risk): fillable depth monotone-down-to-0,
//!     reclaimable >= 0 and == posted at OI==0 — section 5 FillResidue/ReclaimSeed (arithmetic).
//!   * Per-domain insurance budget == seed: disjoint per-market loss bounds, no cross-subsidy — sec 6.
//!   * Thin-rebate sign FIX (A5/A6): is_thin = (is_buy&&inv>=0)||(!is_buy&&inv<=0); total_bps
//!     is a surcharge XOR a rebate (slopes pinned exactly), never crosses oracle (>=0) — section 4.
//!   * Residual depth clip: clip <= opposing depth; pool non-increasing; zero-fill (not panic) on
//!     empty pool — section 4 `check_residual_depth`.
//!
//! WHAT IS *NOT* MODEL-TESTABLE HERE — deferred to the REAL-ENGINE LiteSVM gate (spec sec 9 step 6,
//! sec 10 caveats). The full deferral set, named explicitly so it is auditable:
//!   (1) Taker routing co-sign-vs-relay (the spec BLOCKER, section 9 step 1) — tx/account construction.
//!   (2) Real on-chain signer / authorization gates and counterparty-account binding
//!       (the on-chain account checks) — not expressible in this economic model.
//!   (3) Per-market insurance isolation END-TO-END / cross-market non-contamination on-chain (group
//!       PDA, zero prior vault/insurance, A3) — the model only covers the disjoint-budget arithmetic.
//!   (4) Oracle authenticated-price-from-MARKET (A1), per-slot clamp, RecoveryRequired halt,
//!       externality fee, mark_min_fee wash-resistance, oracle-manipulation cost, staleness/conf
//!       filters, L0-L4 circuit breakers — on-chain oracle plumbing.
//!   (5) Keeper liveness + permissionless SettleResidue liveness + on-chain funding-sign assert.
//!   (6) backing-expiry == market_expiry_slot lifecycle + early wind-down ACCOUNT effects.
//!   (7) Matcher version migration that does not break existing deployed markets.
//!   (8) Real-engine settlement internals: liens, the bound-vs-exact split, the SEPARATE insurance
//!       term in available_backing (which can pay a winner MORE than the losing pool), and the REAL
//!       lockstep account force-close (the model proves only the equal-leg-reduction + conservation
//!       arithmetic, not the account plumbing).
//! NOTE: as in the base model, "loss bounded by the seed/opposing pool" is a property of THIS
//! single-collateral-term model; the real engine's extra insurance term means the on-chain bound
//! differs and must be re-proven in LiteSVM.

use super::{credit_rate, pool_draw, BPS, SCALE};

/// Funding fixed-point scale: `funding_rate_e9` is in 1e9 units (matches engine's e9 rate).
pub const E9: u128 = 1_000_000_000;

/// Integer square root (Newton's method, exact floor). No float. `isqrt(x)*isqrt(x) <= x`
/// and `(isqrt(x)+1)^2 > x`.
pub fn isqrt(x: u128) -> u128 {
    if x < 2 {
        return x;
    }
    // Initial guess from bit length.
    let mut s = {
        let bits = 128 - x.leading_zeros();
        1u128 << ((bits + 1) / 2)
    };
    loop {
        let t = (s + x / s) / 2;
        if t >= s {
            break;
        }
        s = t;
    }
    // s is now floor(sqrt(x)) or one too high; correct down then verify.
    while s.saturating_mul(s) > x {
        s -= 1;
    }
    s
}

// =============================================================================
// 1 + OI CAP SIZED TO SEED  (spec section 6)
// =============================================================================

/// The OI cap (in collateral atoms of notional) sized to the seed, taking the TIGHTER of:
///   * the 1:1 seed-solvency bound `N_cap = Seed`, and
///   * the oracle-gate bound `cap_oi = Seed * 1e4 / Delta_max_bps`
/// where `Delta_max_bps = max_price_move_bps_per_slot * dt_react_slots`.
///
/// Worked (spec): Seed=$10k, move=25bps/slot, dt=50 -> Delta_max=1250bps -> oracle-gate=$80k,
/// binding cap = min(10k, 80k) = $10k (the 1:1 bound binds). A *bigger* Delta_max makes the
/// oracle-gate tighter (smaller), so it can become the binding side for very volatile markets.
///
/// `delta_max_bps == 0` is treated as "no oracle gate" -> falls back to the 1:1 seed bound.
pub fn oi_cap_sized_to_seed(seed_atoms: u128, delta_max_bps: u128) -> u128 {
    if delta_max_bps == 0 {
        return seed_atoms; // no price-move gate -> 1:1 seed bound only
    }
    let oracle_gate = seed_atoms.saturating_mul(BPS) / delta_max_bps;
    core::cmp::min(seed_atoms, oracle_gate)
}

/// Convenience: build Delta_max_bps from the two oracle-tier knobs.
pub fn delta_max_bps(max_price_move_bps_per_slot: u128, dt_react_slots: u128) -> u128 {
    max_price_move_bps_per_slot.saturating_mul(dt_react_slots)
}

/// matcher `max_inventory_abs` derived from the seed-sized OI cap: `N_cap * 1e6 / oracle`
/// (notional cap in atoms -> base/Q units at the oracle price). `oracle_e6 == 0` -> 0
/// (no inventory allowed; matches the spec's "oracle==0 errors / max_inventory_abs==0 closes
/// the silent-unlimited loophole").
pub fn max_inventory_abs_from_cap(n_cap: u128, oracle_e6: u128) -> u128 {
    if oracle_e6 == 0 {
        return 0;
    }
    n_cap.saturating_mul(SCALE) / oracle_e6
}

// =============================================================================
// 2 + SEED LOSS UNDER THE CAP  (spec section 1/6: loss <= seed, ALWAYS)
// =============================================================================

/// The crowd's gross winning CLAIM on the seed for a `r_bps` move on a `n_cap` capped notional.
/// This is the RAW (un-haircut) demand: `n_cap * r / BPS`. It is NOT clamped to the seed — so a
/// loose cap (`n_cap > seed`) genuinely produces `raw_claim > seed`. The whole point of the OI
/// cap is to keep THIS quantity <= seed when the 1:1 bound binds.
pub fn raw_claim_on_seed(n_cap: u128, r_bps: u128) -> u128 {
    n_cap.saturating_mul(r_bps) / BPS
}

/// The seed's realized loss when the crowd wins, given OI is capped at `n_cap`.
///
/// The crowd's winning claim is `n_cap * r` (PnL on the capped one-sided notional). The seed
/// is the counterparty and pays from its posted collateral; by construction its realized loss is
/// `pool_draw(claim, seed) = min(claim, seed) * credit_rate / SCALE <= seed`. NOTE: this <= seed
/// bound follows from `pool_draw` clamping ALONE and is TRUE for any `n_cap` — it is NOT the
/// theorem that justifies the cap. The cap's real job is to keep the RAW claim (see
/// `raw_claim_on_seed`) within the seed so the seed never takes a haircut on the WINNER side and
/// the winner is paid in full. The cap-binding theorem lives in `proof_oi_cap_full_payout` /
/// `proof_oi_cap_raw_claim_bounded`.
///
/// Returns the seed's realized loss (<= seed_atoms by pool_draw clamping).
pub fn seed_loss_on_position(seed_atoms: u128, n_cap: u128, r_bps: u128) -> u128 {
    let claim = raw_claim_on_seed(n_cap, r_bps); // crowd PnL on the capped notional
    pool_draw(claim, seed_atoms)
}

// =============================================================================
// 3 + PAID-LP BREAK-EVEN  (spec section 8)
// =============================================================================

/// PnL of the paid seed/LP over a holding window, in SCALE (1e6) fractional units of the
/// notional. Implements the spec break-even:
///   income  = (fee_bps + spread_bps)/1e4  +  (funding_rate_e9/1e9) * tau_slots
///   adverse ~= (sigma_slot_bps/1e4) * isqrt(tau_slots) * 0.8
/// "Paid enough" iff `income >= adverse`.
///
/// All three returned values are FRACTIONS in SCALE units (1e6 == 100% of notional), so the
/// caller can multiply by notional to get atoms. `net = income - adverse` is SIGNED via
/// (positive, magnitude). 0.8 is modeled as the exact integer ratio 4/5.
pub struct LpPnl {
    pub income: u128,      // SCALE-fraction earned per unit notional
    pub adverse: u128,     // SCALE-fraction lost to adverse selection per unit notional
    pub net_positive: bool,
    pub net_mag: u128,     // |income - adverse| in SCALE units
}

pub fn paid_lp_pnl(
    fee_bps: u128,
    spread_bps: u128,
    funding_rate_e9: u128,
    tau_slots: u128,
    sigma_slot_bps: u128,
    _n_notional: u128,
) -> LpPnl {
    LpPnl::compute(fee_bps, spread_bps, funding_rate_e9, tau_slots, sigma_slot_bps)
}

impl LpPnl {
    /// The closed form, in SCALE units, exposed so proofs/tests can recompute it independently
    /// (catching slope/coefficient regressions). PRECISION FIX (AF): funding income multiplies by
    /// tau BEFORE dividing by E9 so spec-range rates (funding_rate_e9 in [100,3000]) do not
    /// truncate to 0 per slot. The 0.8 adverse coefficient is the exact integer ratio 4/5.
    pub fn closed_form_income(fee_bps: u128, spread_bps: u128, funding_rate_e9: u128, tau_slots: u128) -> u128 {
        // turnover: (fee+spread) bps -> SCALE: bps * SCALE / BPS.
        let turnover_income = (fee_bps.saturating_add(spread_bps)).saturating_mul(SCALE) / BPS;
        // funding total over the window in SCALE units: (rate_e9 * tau / 1e9) scaled to SCALE.
        // Multiply rate_e9 * tau FIRST (full precision), THEN convert e9 -> SCALE in one divide.
        // (rate_e9 * tau / E9) is in "1.0 == 1" fractional units; * SCALE puts it in SCALE units.
        // Combined: rate_e9 * tau * SCALE / E9. Since SCALE=1e6, E9=1e9 => factor /1000, but we
        // keep the tau multiply ahead of every divide to avoid the per-slot cliff.
        let funding_income = funding_rate_e9
            .saturating_mul(tau_slots)
            .saturating_mul(SCALE)
            / E9;
        turnover_income.saturating_add(funding_income)
    }

    /// adverse = (sigma_slot_bps/1e4) * sqrt(tau) * 0.8  in SCALE units, as an exact integer:
    /// sigma_slot_bps * SCALE * isqrt(tau) * 4 / (BPS * 5). Multiplies before dividing.
    pub fn closed_form_adverse(sigma_slot_bps: u128, tau_slots: u128) -> u128 {
        sigma_slot_bps
            .saturating_mul(SCALE)
            .saturating_mul(isqrt(tau_slots))
            .saturating_mul(4)
            / (BPS.saturating_mul(5))
    }

    pub fn compute(fee_bps: u128, spread_bps: u128, funding_rate_e9: u128, tau_slots: u128, sigma_slot_bps: u128) -> LpPnl {
        let income = Self::closed_form_income(fee_bps, spread_bps, funding_rate_e9, tau_slots);
        let adverse = Self::closed_form_adverse(sigma_slot_bps, tau_slots);
        if income >= adverse {
            LpPnl { income, adverse, net_positive: true, net_mag: income - adverse }
        } else {
            LpPnl { income, adverse, net_positive: false, net_mag: adverse - income }
        }
    }
}

// =============================================================================
// 4 + ZERO-SUM FUNDING WHOSE SIGN OPPOSES THE IMBALANCE  (spec section 5/8)
// =============================================================================

/// Result of a funding transfer between the long and short collateral pools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FundingTransfer {
    pub new_long: u128,
    pub new_short: u128,
    pub amount: u128,      // atoms moved
    pub long_pays: bool,   // true => long paid short; false => short paid long
}

/// Apply a funding transfer. The MAGNITUDE is `rate_e9` applied to the smaller pool (so the
/// payer can always cover it — zero-sum, no insolvency). The SIGN MUST oppose the imbalance:
/// the crowded (larger) pool pays the thin (smaller) pool. A caller passing a wrong-sign
/// intent is rejected (returns None) — the model refuses to move funding the wrong way.
///
/// `want_long_pays`: the caller's claimed direction. We accept it ONLY if it matches the
/// imbalance-opposing direction (long_pays iff m_long > m_short). Balanced => no transfer.
pub fn funding_transfer(
    m_long: u128,
    m_short: u128,
    rate_e9: u128,
    want_long_pays: bool,
) -> Option<FundingTransfer> {
    if m_long == m_short {
        // Balanced: only the no-op "transfer" is valid; sign is irrelevant but amount must be 0.
        return Some(FundingTransfer {
            new_long: m_long,
            new_short: m_short,
            amount: 0,
            long_pays: want_long_pays,
        });
    }
    let correct_long_pays = m_long > m_short; // crowded side pays thin side
    if want_long_pays != correct_long_pays {
        return None; // WRONG SIGN -> reject (would push the book further out of balance)
    }
    // Magnitude: rate applied to the PAYER pool, capped at the payer so it's always coverable.
    let (payer, payee) = if correct_long_pays { (m_long, m_short) } else { (m_short, m_long) };
    let raw = payer.saturating_mul(rate_e9) / E9;
    let amount = core::cmp::min(raw, payer); // never move more than the payer holds
    let new_payer = payer - amount;
    let new_payee = payee.saturating_add(amount);
    let (new_long, new_short) = if correct_long_pays { (new_payer, new_payee) } else { (new_payee, new_payer) };
    Some(FundingTransfer {
        new_long,
        new_short,
        amount,
        long_pays: correct_long_pays,
    })
}

// =============================================================================
// 5 + SEED EXHAUSTION: credit_rate falls monotonically, never insolvent  (section 1/2)
// =============================================================================

/// One settlement of a winning claim against draining backing. Returns (out, new_backing).
/// `out <= in_claim` (winner never overpaid) AND `out <= backing` (never insolvent).
pub fn drain_step(backing: u128, claim: u128) -> (u128, u128) {
    let out = pool_draw(claim, backing); // <= min(claim, backing)
    let new_backing = backing - out; // out <= backing guarantees no underflow
    (out, new_backing)
}

/// Drive a sequence of equal winning claims against a fixed initial backing (the seed) and
/// record the credit_rate trajectory. Models seed exhaustion: as backing drains, credit_rate
/// falls MONOTONICALLY toward 0, total paid out <= seed (never insolvent), and the seed loses
/// at most the seed (its whole posted backing).
pub struct DrainOutcome {
    pub total_out: u128,        // total paid to winners (<= initial backing)
    pub final_backing: u128,    // what the seed has left (>= 0)
    pub final_credit_rate: u128,// credit_rate of the LAST nonzero-claim step
    pub min_credit_rate: u128,  // lowest credit_rate seen (the floor reached)
    pub monotone: bool,         // credit_rate never increased step-to-step
}

pub fn drain_sequence(initial_backing: u128, claim_per_step: u128, steps: u32) -> DrainOutcome {
    let mut backing = initial_backing;
    let mut total_out = 0u128;
    let mut prev_cr = SCALE; // start "fully funded"
    let mut min_cr = SCALE;
    let mut last_cr = SCALE;
    let mut monotone = true;
    for _ in 0..steps {
        let cr = credit_rate(backing, claim_per_step);
        if cr > prev_cr {
            monotone = false;
        }
        let (out, nb) = drain_step(backing, claim_per_step);
        total_out = total_out.saturating_add(out);
        backing = nb;
        prev_cr = cr;
        last_cr = cr;
        if cr < min_cr {
            min_cr = cr;
        }
    }
    DrainOutcome {
        total_out,
        final_backing: backing,
        final_credit_rate: last_cr,
        min_credit_rate: min_cr,
        monotone,
    }
}

// =============================================================================
// 6 + THIN-REBATE (sign FIX A5/A6) + surcharge XOR rebate, never crosses oracle  (section 4)
// =============================================================================

/// The execution-price adjustment, in bps, for a residual fill. Composed of a base spread,
/// PLUS a skew surcharge when the fill WORSENS the seed's inventory, OR a rebate when the fill
/// IMPROVES it (thin side). The FIXED is_thin condition (A5/A6):
///   is_thin = (is_buy && inv >= 0) || (!is_buy && inv <= 0)
/// i.e. the complement of the skew 'worsens' branch. A fill is EITHER a surcharge (worsens) OR
/// a rebate (thins) — never both. `total_bps` is clamped to be >= 0 so exec_price never crosses
/// the oracle (a rebate can shave the spread but cannot flip the sign).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThinAdj {
    pub total_bps: u128,    // final bps applied (>= 0 always)
    pub surcharge_bps: u128,// skew surcharge (0 if this is a rebate fill)
    pub rebate_bps: u128,   // rebate (0 if this is a surcharge fill)
    pub is_thin: bool,
}

/// `inv` is the seed's signed inventory (Q-units): represented as (inv_abs, inv_is_long).
/// inv >= 0 means seed is net long (or flat). The spec uses a single signed integer; we model
/// the sign explicitly to stay in u128.
///
/// `mult_bps` = skew_rebate_mult_bps (new field A11). `base_spread_bps` = the base spread that
/// a surcharge adds to and a rebate is capped by.
pub fn thin_rebate_bps(
    inv_abs: u128,
    inv_is_long: bool, // inv >= 0
    is_buy: bool,
    mult_bps: u128,
    base_spread_bps: u128,
) -> ThinAdj {
    // is_buy && inv>=0  -> seed already long, a buy makes it MORE long? No: a residual BUY by a
    // taker means the SEED SELLS (takes the short side), thinning a long inventory -> THIN.
    // We follow the spec's stated condition verbatim (A5/A6 fix):
    let inv_nonneg = inv_is_long || inv_abs == 0; // inv >= 0
    let inv_nonpos = !inv_is_long || inv_abs == 0; // inv <= 0
    let is_thin = (is_buy && inv_nonneg) || (!is_buy && inv_nonpos);

    if is_thin {
        // REBATE: shave the spread proportional to |inv|, capped at base_spread (can't go below 0).
        let rebate = if mult_bps == 0 {
            0
        } else {
            core::cmp::min(inv_abs.saturating_mul(mult_bps) / BPS, base_spread_bps)
        };
        let total = base_spread_bps - rebate; // rebate <= base_spread -> no underflow, >= 0
        ThinAdj { total_bps: total, surcharge_bps: 0, rebate_bps: rebate, is_thin: true }
    } else {
        // SURCHARGE (worsens): add a skew surcharge proportional to |inv| on top of the spread.
        let surcharge = inv_abs.saturating_mul(mult_bps) / BPS;
        let total = base_spread_bps.saturating_add(surcharge);
        ThinAdj { total_bps: total, surcharge_bps: surcharge, rebate_bps: 0, is_thin: false }
    }
}

// =============================================================================
// 7 + RESIDUAL DEPTH CLIP (matcher)  (spec section 4 check_residual_depth)
// =============================================================================

/// The matcher clip: how much of a requested fill can actually be filled against the residual
/// pool. Faithful to `check_residual_depth` (section 4):
///   depth_q = opposing_atoms * 1e6 / oracle_e6   (floor = conservative)
///   cap_q   = min(depth_q, max_inv)
///   return min(fill_abs, cap_q)
/// Zero-fill (NOT panic) on empty opposing pool. Returns 0 (not error) when oracle==0 too here
/// (the on-chain version errors; the model treats it as zero-fill to keep the clip total).
pub fn residual_depth_clip(opposing_atoms: u128, oracle_e6: u128, max_inv: u128, fill_abs: u128) -> u128 {
    if oracle_e6 == 0 {
        return 0; // no price -> no fill (on-chain: Err; model: zero-fill, conservative)
    }
    if opposing_atoms == 0 {
        return 0; // empty pool -> zero-fill, NOT panic
    }
    let depth_q = opposing_atoms.saturating_mul(SCALE) / oracle_e6; // floor
    let cap_q = core::cmp::min(depth_q, max_inv);
    core::cmp::min(fill_abs, cap_q)
}

/// Post-fill pool deduction at the ORACLE price (section 4 post-fill update): the cost in atoms
/// is `|exec_size| * oracle_e6 / 1e6`, deducted (saturating) from the opposing pool. Returns the
/// new opposing-pool size — ALWAYS <= the old size (pool non-increasing).
pub fn deduct_pool_at_oracle(opposing_atoms: u128, exec_size_q: u128, oracle_e6: u128) -> u128 {
    let cost_atoms = exec_size_q.saturating_mul(oracle_e6) / SCALE;
    opposing_atoms.saturating_sub(cost_atoms)
}

// =============================================================================
// 8 + LEVERAGE-RATCHET INITIAL-MARGIN GATE  (spec section 8)
// =============================================================================

/// The leverage ratchet: `initial_margin_bps = max(1000, 1e4 * OI_crowded / N_cap)`, clamped to
/// `<= BPS` (1x at/over the cap). 10x (1000bps) when the seed-facing book is empty, tightening to
/// 1x (BPS) as the one-sided OI fills toward the cap. This is what keeps a LATE lopsided entrant
/// from breaching the seed-loss bound: required margin grows as crowding grows. `n_cap == 0`
/// degenerates to the maximally-conservative 1x gate.
pub fn initial_margin_bps(oi_crowded: u128, n_cap: u128) -> u128 {
    if n_cap == 0 {
        return BPS; // no capacity -> require full (1x) margin
    }
    let ratio = BPS.saturating_mul(oi_crowded) / n_cap;
    let floored = core::cmp::max(1000, ratio); // floor at 10x leverage (1000 bps)
    core::cmp::min(BPS, floored) // cap at 1x leverage (BPS)
}

/// Does the initial-margin gate ADMIT a new position of `notional` posting `equity`, given the
/// current crowded OI and the cap? Mirrors the on-chain initial-margin gate: `equity >= notional *
/// initial_margin_bps / BPS`.
pub fn margin_gate_admits(equity: u128, notional: u128, oi_crowded: u128, n_cap: u128) -> bool {
    let mbps = initial_margin_bps(oi_crowded, n_cap);
    let initial_req = notional.saturating_mul(mbps) / BPS;
    equity >= initial_req
}

// =============================================================================
// 9 + FUNDING SCHEDULE clamp(base + slope*imbalance, 0, max)  (spec section 8)
// =============================================================================

/// The funding-rate schedule (section 8): `funding_rate_e9 = clamp(base + slope*imbalance, 0,
/// max)`. Spec defaults base=100, slope=1500, max=3000 (engine hard max 10_000). `imbalance_frac`
/// is in SCALE units (1e6 == fully one-sided); the slope term is `slope * imbalance_frac / SCALE`.
/// Monotone NON-DECREASING in imbalance, floored at `base` (for imbalance==0), capped at `max`.
pub fn funding_rate_from_imbalance(imbalance_frac: u128, base_e9: u128, slope_e9: u128, max_e9: u128) -> u128 {
    let slope_term = slope_e9.saturating_mul(imbalance_frac) / SCALE;
    let raw = base_e9.saturating_add(slope_term);
    core::cmp::min(raw, max_e9)
}

/// The directed funding decision: produce both the magnitude (schedule above) and the
/// imbalance-opposing direction (`long_pays` iff longs are crowded). A balanced book yields rate
/// at the base but no defined direction (we return long_pays=false, caller treats balanced as no
/// transfer). The (rate, direction) it produces is, BY CONSTRUCTION, never rejected by
/// `funding_transfer` (sign always opposes imbalance).
pub fn funding_decision(m_long: u128, m_short: u128, base_e9: u128, slope_e9: u128, max_e9: u128) -> (u128, bool) {
    let total = m_long.saturating_add(m_short);
    let (imbalance_frac, long_pays) = if total == 0 || m_long == m_short {
        (0u128, false)
    } else if m_long > m_short {
        ((m_long - m_short).saturating_mul(SCALE) / total, true)
    } else {
        ((m_short - m_long).saturating_mul(SCALE) / total, false)
    };
    let rate = funding_rate_from_imbalance(imbalance_frac, base_e9, slope_e9, max_e9);
    (rate, long_pays)
}

// =============================================================================
// 10 + FEE ROUTING 100%-to-seed split  (spec section 4: fee routing)
// =============================================================================

/// Split a trade fee between the per-market insurance fund and the seed. Mirrors
/// the on-chain fee split: `to_insurance = fee * insurance_bps / BPS`, `to_seed =
/// fee - to_insurance`. For the Residual matcher, `fee_to_insurance_bps == 0` so 100% routes to
/// the seed. Conserving by construction (`to_insurance + to_seed == fee`). Returns
/// `(to_insurance, to_seed)`. `fee_to_insurance_bps` is clamped to BPS so to_insurance <= fee.
pub fn split_insurance_fee(fee_atoms: u128, fee_to_insurance_bps: u128) -> (u128, u128) {
    let bps = core::cmp::min(fee_to_insurance_bps, BPS);
    let to_insurance = fee_atoms.saturating_mul(bps) / BPS;
    let to_seed = fee_atoms - to_insurance; // to_insurance <= fee (bps<=BPS) -> no underflow
    (to_insurance, to_seed)
}

// =============================================================================
// 11 + LOCKSTEP CLOSE: OI parity + conservation  (spec section 5)
// =============================================================================

/// Outcome of closing `n_close` notional of BOTH legs in lockstep (the SettleResidue wind-down).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockstepClose {
    pub new_long: u128,      // long collateral after the paired close
    pub new_short: u128,     // short collateral after the paired close
    pub oi_long_after: u128, // long open interest after (Q-units)
    pub oi_short_after: u128,// short open interest after
    pub draw: u128,          // value moved from loser pool to winner pool during the close
}

/// Close `n_close` of each leg simultaneously, realizing a `pnl_bps` move (`up` => longs win).
/// Both legs reduce by the SAME notional (OI parity): starting from matched OI
/// `oi == oi_long == oi_short`, after the close `oi_long_after == oi_short_after`. Total
/// collateral is conserved (only redistributed by `pool_draw`, never created/destroyed). The
/// losing leg's pool never pays more than it holds (reuses `pool_draw`).
pub fn lockstep_close(m_long: u128, m_short: u128, oi: u128, n_close: u128, pnl_bps: u128, up: bool) -> LockstepClose {
    let close = core::cmp::min(n_close, oi); // can't close more OI than exists
    let claim = close.saturating_mul(pnl_bps) / BPS; // realized PnL on the closed notional
    let (m_w, m_l) = if up { (m_long, m_short) } else { (m_short, m_long) };
    let draw = pool_draw(claim, m_l); // <= m_l: loser leg never overpays
    let (nw, nl) = (m_w.saturating_add(draw), m_l - draw);
    let (new_long, new_short) = if up { (nw, nl) } else { (nl, nw) };
    let oi_after = oi - close; // SAME reduction on BOTH legs -> parity preserved
    LockstepClose {
        new_long,
        new_short,
        oi_long_after: oi_after,
        oi_short_after: oi_after,
        draw,
    }
}

// =============================================================================
// 12 + SEED ACCOUNT: posted vs realized-loss / at-risk  (spec section 5 FillResidue/ReclaimSeed)
// =============================================================================

/// Two-accumulator seed state: `posted` (total collateral the seed put up) vs `realized_loss`
/// (what has been drawn away by winning crowds) and `at_risk` (currently encumbered by open OI).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeedAccount {
    pub posted: u128,
    pub realized_loss: u128, // monotone non-decreasing, <= posted
    pub at_risk: u128,       // currently encumbered (open OI backing)
}

impl SeedAccount {
    pub fn new(posted: u128) -> Self {
        SeedAccount { posted, realized_loss: 0, at_risk: 0 }
    }

    /// The remaining fillable depth in Q-units at `oracle_e6`: `(posted - realized_loss) * 1e6 /
    /// oracle` (FillResidue clip). Monotone NON-INCREASING as realized_loss accrues; reaches 0
    /// exactly when realized_loss == posted (seed exhausted -> no further residual fills, matching
    /// credit_rate -> 0). Zero-fill (not panic) on oracle==0.
    pub fn fillable_depth(&self, oracle_e6: u128) -> u128 {
        if oracle_e6 == 0 {
            return 0;
        }
        let remaining = self.posted.saturating_sub(self.realized_loss);
        remaining.saturating_mul(SCALE) / oracle_e6
    }

    /// Reclaimable seed = `posted - at_risk` (ReclaimSeed). Never negative (saturating). Equals
    /// `posted` once OI winds down (at_risk == 0) for a clean reclaim.
    pub fn reclaimable(&self) -> u128 {
        self.posted.saturating_sub(self.at_risk)
    }

    /// Accrue a realized loss (a winning crowd drew from the seed), clamped so it never exceeds
    /// posted. Returns the actual loss booked (<= what was requested, never pushing past posted).
    pub fn book_loss(&mut self, loss: u128) -> u128 {
        let room = self.posted - self.realized_loss; // realized_loss <= posted invariant
        let booked = core::cmp::min(loss, room);
        self.realized_loss += booked;
        booked
    }
}

// =============================================================================
// 13 + PER-DOMAIN INSURANCE BUDGET == seed  (spec section 6: disjoint per-market bounds)
// =============================================================================

/// Per-market insurance domain budget, set to the seed at activation (`insurance_domain_budget =
/// seed_atoms`). The realizable winner draw against THIS market is bounded by THIS market's own
/// budget — there is no shared/global term, so two independent markets' loss bounds add
/// disjointly and a market with budget 0 cannot subsidize another.
pub fn domain_budget_at_activation(seed_atoms: u128) -> u128 {
    seed_atoms
}

/// The realizable winner draw against a single market's own domain budget for a crowd claim.
/// Bounded by `budget` (its own seed), never a shared pool.
pub fn realizable_draw_in_domain(claim: u128, budget: u128) -> u128 {
    pool_draw(claim, budget)
}

// =============================================================================
// FORMAL PROOFS (Kani). Each asserts an invariant AND covers the binding regime non-vacuously.
// =============================================================================
#[cfg(kani)]
mod proofs {
    use super::*;

    /// PROOF (CAP IS LOAD-BEARING) — the RAW crowd claim (un-haircut: `n_cap*r/BPS`) on a
    /// modest in-tier move is bounded by the seed ONLY because `n_cap <= seed`. This is the
    /// theorem that justifies the cap: we feed `n_cap` from `oi_cap_sized_to_seed` (which the
    /// proof also pins at `<= seed`) and assert the raw claim — with NO pool_draw clamp — stays
    /// within the seed for a move within the 1:1 window (r <= BPS). A loose cap (n_cap > seed)
    /// would break this even at r==BPS. Mutation S/AF target: a cap function returning >seed.
    #[kani::proof]
    fn proof_oi_cap_raw_claim_bounded() {
        let seed: u128 = kani::any();
        kani::assume(seed >= 1 && seed <= 1_000_000_000);
        let delta_max: u128 = kani::any();
        kani::assume(delta_max <= 1_000_000); // up to a big oracle-gate window

        let n_cap = oi_cap_sized_to_seed(seed, delta_max);
        // The cap function never exceeds the 1:1 seed bound. This is load-bearing.
        assert!(n_cap <= seed);

        // For a move within the 1:1 window (r <= BPS, i.e. <= 100%), the RAW (un-haircut) claim
        // is bounded by the seed BECAUSE n_cap <= seed. (raw = n_cap*r/BPS <= n_cap <= seed.)
        let r_bps: u128 = kani::any();
        kani::assume(r_bps <= BPS); // in-tier 1:1 window
        let raw = raw_claim_on_seed(n_cap, r_bps);
        assert!(raw <= seed); // THEOREM: capped OI keeps the raw claim within the seed

        // NON-VACUITY: oracle-gate is the binding (tighter) side sometimes (n_cap < seed)...
        kani::cover!(delta_max > BPS && n_cap < seed);
        // ...and the 1:1 bound binds other times (n_cap == seed)...
        kani::cover!(delta_max <= BPS && delta_max > 0 && n_cap == seed);
        // ...and the raw claim is pinned at the full seed (the bound is TIGHT, not slack)...
        kani::cover!(raw == seed && seed > 0);
        // ...and a strictly-under-seed raw claim is reachable.
        kani::cover!(raw > 0 && raw < seed);
    }

    /// PROOF (FULL-PAYOUT SOLVENCY) — with `n_cap` from the cap and a move within the 1:1 window
    /// (r <= BPS), the raw claim <= seed, so `credit_rate(seed, claim) == SCALE`: the WINNER is
    /// paid 100%, no haircut. This is the cap's REAL job per section 6 (keep the winner whole),
    /// distinct from the trivial `pool_draw <= seed` clamp.
    #[kani::proof]
    fn proof_oi_cap_full_payout() {
        let seed: u128 = kani::any();
        kani::assume(seed >= 1 && seed <= 1_000_000_000);
        let delta_max: u128 = kani::any();
        kani::assume(delta_max <= 1_000_000);
        let n_cap = oi_cap_sized_to_seed(seed, delta_max);

        let r_bps: u128 = kani::any();
        kani::assume(r_bps <= BPS); // within the 1:1 window
        let claim = raw_claim_on_seed(n_cap, r_bps);
        // Winner paid in full: credit_rate is 1.0 and the realized loss equals the claim exactly.
        assert!(credit_rate(seed, claim) == SCALE);
        assert!(seed_loss_on_position(seed, n_cap, r_bps) == claim); // out == claim (no haircut)

        kani::cover!(claim > 0 && claim == seed);   // full-seed claim still paid in full
        kani::cover!(claim > 0 && claim < seed);    // partial claim paid in full
    }

    /// PROOF (REALIZED-LOSS CLAMP) — independent of the cap: the realized loss `pool_draw(claim,
    /// seed)` is ALWAYS <= seed for ANY move (this follows from pool_draw clamping, NOT the cap;
    /// we state it honestly as a separate, weaker fact). Covers the over-the-window regime where
    /// the WINNER takes a haircut (credit_rate < SCALE) — exactly the haircut UX the spec warns of.
    #[kani::proof]
    fn proof_seed_realized_loss_clamped() {
        let seed: u128 = kani::any();
        kani::assume(seed <= 1_000_000_000);
        let n_cap: u128 = kani::any();
        kani::assume(n_cap <= 1_000_000_000);
        let r_bps: u128 = kani::any();
        kani::assume(r_bps <= 1_000_000); // absurdly large move (>100%): winner CAN be haircut

        let loss = seed_loss_on_position(seed, n_cap, r_bps);
        assert!(loss <= seed); // realized loss bounded by seed (pool_draw clamp)

        // NON-VACUITY: a winner haircut (claim > seed) actually occurs in the over-window regime.
        let claim = raw_claim_on_seed(n_cap, r_bps);
        kani::cover!(claim > seed && loss == seed);            // winner haircut, loss pinned at seed
        kani::cover!(claim > 0 && claim <= seed && loss == claim); // funded, paid in full
    }

    /// PROOF — seed exhaustion: drain => credit_rate non-increasing, out <= in, winner never
    /// overpaid, never insolvent.
    #[kani::proof]
    fn proof_credit_rate_monotone_no_insolvency() {
        let backing: u128 = kani::any();
        kani::assume(backing <= 1_000_000_000);
        let claim: u128 = kani::any();
        kani::assume(claim >= 1 && claim <= 1_000_000_000);

        let cr0 = credit_rate(backing, claim);
        let (out, nb) = drain_step(backing, claim);
        // LOAD-BEARING EXACT VALUE: out is the credit-rate-discounted claim, NOT just min(claim,
        // backing). This catches the insolvency mutation (out = min(claim, backing) overpays the
        // winner vs the haircut: e.g. backing=7, claim=999999 -> true pays 6, the bug pays 7).
        assert!(out == claim.saturating_mul(cr0) / SCALE);
        // out <= claim (winner never overpaid) AND out <= backing (never insolvent).
        assert!(out <= claim);
        assert!(out <= backing);
        assert!(nb == backing - out);
        // credit_rate after the drain is NON-INCREASING (backing only shrank).
        let cr1 = credit_rate(nb, claim);
        assert!(cr1 <= cr0); // THEOREM: monotone non-increasing as backing drains

        kani::cover!(cr0 == SCALE && cr1 < SCALE); // funded -> falls into haircut (binding regime)
        kani::cover!(out > 0 && out < claim);      // haircut payout actually occurs
        kani::cover!(nb == 0 && backing > 0);      // full exhaustion reachable
    }

    /// PROOF — funding conserves total AND its sign opposes the imbalance; wrong sign rejected;
    /// the MAGNITUDE is exactly `min(payer*rate/E9, payer)` (the payer-cap is load-bearing).
    #[kani::proof]
    fn proof_funding_zero_sum_and_sign() {
        let m_long: u128 = kani::any();
        kani::assume(m_long <= 1_000_000_000);
        let m_short: u128 = kani::any();
        kani::assume(m_short <= 1_000_000_000);
        // RATE BOUND RAISED past E9 (up to 500%) so the payer-cap actually fires (else the
        // min(raw, payer) clamp is dead code; a rate > E9 would otherwise underflow `payer-amount`).
        let rate_e9: u128 = kani::any();
        kani::assume(rate_e9 <= 5 * E9);
        let want: bool = kani::any();

        let before = m_long.saturating_add(m_short);
        match funding_transfer(m_long, m_short, rate_e9, want) {
            Some(t) => {
                // ZERO-SUM: total collateral conserved exactly.
                assert!(t.new_long.saturating_add(t.new_short) == before);
                // SIGN + EXACT MAGNITUDE: if any value moved, the payer is the crowded side and
                // the amount is exactly the payer-capped schedule (catches payer/payee swap and
                // a removed cap).
                if t.amount > 0 {
                    let payer = if t.long_pays { m_long } else { m_short };
                    if t.long_pays {
                        assert!(m_long > m_short); // long paid => long was crowded
                    } else {
                        assert!(m_short > m_long); // short paid => short was crowded
                    }
                    let expected = core::cmp::min(payer.saturating_mul(rate_e9) / E9, payer);
                    assert!(t.amount == expected);        // EXACT magnitude tied to the PAYER pool
                    assert!(t.amount <= payer);           // never more than the payer holds (no underflow)
                }
                // never moved more than a side could cover (no negative pool).
                assert!(t.new_long <= before && t.new_short <= before);
            }
            None => {
                // Only a wrong-sign request on an IMBALANCED book is rejected.
                assert!(m_long != m_short);
                let correct = m_long > m_short;
                assert!(want != correct);
            }
        }

        kani::cover!(funding_transfer(m_long, m_short, rate_e9, want).is_none()); // rejection reachable
        kani::cover!(matches!(funding_transfer(m_long, m_short, rate_e9, want), Some(t) if t.amount > 0));
        // NON-VACUITY: the payer-cap actually BINDS (rate > E9 -> amount == payer, the whole pool).
        kani::cover!(matches!(funding_transfer(m_long, m_short, rate_e9, want),
            Some(t) if t.amount > 0 && t.amount == if t.long_pays { m_long } else { m_short }));
    }

    /// PROOF — thin-rebate: total_bps >= 0 always (never crosses oracle); surcharge and rebate
    /// are never both nonzero (XOR).
    #[kani::proof]
    fn proof_thin_rebate_no_cross_oracle() {
        let inv_abs: u128 = kani::any();
        kani::assume(inv_abs <= 1_000_000_000);
        let inv_is_long: bool = kani::any();
        let is_buy: bool = kani::any();
        let mult_bps: u128 = kani::any();
        kani::assume(mult_bps <= 100_000);
        let base_spread_bps: u128 = kani::any();
        kani::assume(base_spread_bps <= 10_000);

        let a = thin_rebate_bps(inv_abs, inv_is_long, is_buy, mult_bps, base_spread_bps);
        // total_bps is u128 so >= 0 by type; the LOAD-BEARING claim is it never UNDERFLOWED
        // below the spread by more than the spread (rebate <= base_spread). Assert the rebate cap.
        assert!(a.rebate_bps <= base_spread_bps);
        // XOR: never both a surcharge AND a rebate.
        assert!(!(a.surcharge_bps > 0 && a.rebate_bps > 0));
        // EXACT SLOPE (recomputed FROM INPUTS, not from outputs): catches a doubled/halved slope.
        let uncapped = inv_abs.saturating_mul(mult_bps) / BPS; // the A11 skew_rebate_mult slope term
        // total reconstructs from the components and stays >= 0 (within the spread for a rebate).
        if a.is_thin {
            assert!(a.surcharge_bps == 0);
            // rebate is the slope term capped at the spread.
            assert!(a.rebate_bps == core::cmp::min(uncapped, base_spread_bps));
            assert!(a.total_bps == base_spread_bps - a.rebate_bps);
            assert!(a.total_bps <= base_spread_bps);
        } else {
            assert!(a.rebate_bps == 0);
            // surcharge is the slope term EXACTLY (no cap on the surcharge side).
            assert!(a.surcharge_bps == uncapped);
            assert!(a.total_bps == base_spread_bps.saturating_add(a.surcharge_bps));
            assert!(a.total_bps >= base_spread_bps);
        }

        kani::cover!(a.is_thin && a.rebate_bps > 0);     // a real rebate occurs
        kani::cover!(!a.is_thin && a.surcharge_bps > 0); // a real surcharge occurs
        kani::cover!(a.is_thin && a.total_bps == 0);     // rebate fully consumes the spread (=0, not <0)
        kani::cover!(a.is_thin && a.rebate_bps < uncapped); // the rebate CAP actually binds (slope > spread)
    }

    /// PROOF — depth clip is safe: clip <= opposing depth; pool non-increasing; no panic; zero-fill
    /// on empty pool.
    #[kani::proof]
    fn proof_depth_clip_safe() {
        let opposing: u128 = kani::any();
        kani::assume(opposing <= 1_000_000_000_000);
        let oracle: u128 = kani::any();
        kani::assume(oracle <= 1_000_000_000);
        let max_inv: u128 = kani::any();
        kani::assume(max_inv <= 1_000_000_000_000);
        let fill_abs: u128 = kani::any();
        kani::assume(fill_abs <= 1_000_000_000_000);

        let clip = residual_depth_clip(opposing, oracle, max_inv, fill_abs);
        assert!(clip <= fill_abs);   // never fills more than requested
        assert!(clip <= max_inv);    // honors the OI cap
        if opposing == 0 || oracle == 0 {
            assert!(clip == 0);      // empty pool / no price -> zero-fill (not panic)
        } else {
            // clip never exceeds the available depth in Q-units.
            let depth_q = opposing.saturating_mul(SCALE) / oracle;
            assert!(clip <= depth_q);
        }

        // POOL NON-INCREASING after deducting the executed size at oracle.
        if oracle > 0 {
            let new_pool = deduct_pool_at_oracle(opposing, clip, oracle);
            assert!(new_pool <= opposing);
        }

        kani::cover!(clip > 0 && clip == max_inv);   // OI cap binds
        kani::cover!(clip > 0 && clip == fill_abs);  // request binds (fully filled)
        kani::cover!(opposing > 0 && oracle > 0 && clip == 0); // depth too thin -> zero-fill
    }

    /// PROOF — a multi-step path of settlements + funding conserves total collateral.
    #[kani::proof]
    fn proof_conservation_multistep_with_funding() {
        let mut m_long: u128 = kani::any();
        kani::assume(m_long >= 1 && m_long <= 1_000_000_000);
        let mut m_short: u128 = kani::any();
        kani::assume(m_short >= 1 && m_short <= 1_000_000_000);
        let n: u128 = kani::any();
        kani::assume(n >= 1 && n <= 1_000_000_000);
        let total = m_long.saturating_add(m_short);

        // Step 1: a settlement (longs win some move).
        let r1: u128 = kani::any();
        kani::assume(r1 <= 10_000);
        let claim1 = n.saturating_mul(r1) / BPS;
        let draw1 = pool_draw(claim1, m_short);
        m_long = m_long.saturating_add(draw1);
        m_short -= draw1;
        assert!(m_long.saturating_add(m_short) == total); // conserved

        // Step 2: a funding transfer (sign forced to oppose the imbalance).
        let rate_e9: u128 = kani::any();
        kani::assume(rate_e9 <= E9);
        let want = m_long > m_short; // correct sign
        if let Some(t) = funding_transfer(m_long, m_short, rate_e9, want) {
            m_long = t.new_long;
            m_short = t.new_short;
        }
        assert!(m_long.saturating_add(m_short) == total); // conserved after funding too

        // Step 3: a settlement the other way.
        let r3: u128 = kani::any();
        kani::assume(r3 <= 10_000);
        let claim3 = n.saturating_mul(r3) / BPS;
        let draw3 = pool_draw(claim3, m_long);
        m_short = m_short.saturating_add(draw3);
        m_long -= draw3;
        assert!(m_long.saturating_add(m_short) == total); // THEOREM: conserved end to end

        kani::cover!(draw1 > 0 && draw3 > 0); // both settlements actually moved value
    }

    /// PROOF (LEVERAGE RATCHET) — initial_margin_bps is monotone NON-DECREASING in crowded OI,
    /// floored at 1000 (10x) and capped at BPS (1x). A position pushing seed-facing OI past N_cap
    /// is required to post FULL margin (1x). The gate rejects an under-margined late entrant.
    #[kani::proof]
    fn proof_leverage_ratchet_monotone() {
        let n_cap: u128 = kani::any();
        kani::assume(n_cap >= 1 && n_cap <= 1_000_000_000);
        let oi_a: u128 = kani::any();
        kani::assume(oi_a <= 2_000_000_000);
        let oi_b: u128 = kani::any();
        kani::assume(oi_b <= 2_000_000_000);

        let m_a = initial_margin_bps(oi_a, n_cap);
        let m_b = initial_margin_bps(oi_b, n_cap);
        // Range: floored at 10x, capped at 1x.
        assert!(m_a >= 1000 && m_a <= BPS);
        // MONOTONE: more crowding never lowers the required margin.
        if oi_a <= oi_b {
            assert!(m_a <= m_b);
        }
        // At/over the cap the gate demands full (1x) margin.
        if oi_a >= n_cap {
            assert!(m_a == BPS);
        }

        // Gate admits iff equity covers notional*margin/BPS.
        let equity: u128 = kani::any();
        kani::assume(equity <= 1_000_000_000_000);
        let notional: u128 = kani::any();
        kani::assume(notional <= 1_000_000_000);
        let admits = margin_gate_admits(equity, notional, oi_a, n_cap);
        let req = notional.saturating_mul(m_a) / BPS;
        assert!(admits == (equity >= req));

        kani::cover!(m_a == 1000 && oi_a < n_cap);            // empty/thin book -> 10x floor
        kani::cover!(m_a == BPS && oi_a >= n_cap);            // at-cap -> 1x
        kani::cover!(m_a > 1000 && m_a < BPS);                // mid-ratchet (strictly between)
        kani::cover!(oi_a < oi_b && m_a < m_b);               // ratchet strictly increases
        kani::cover!(!admits && req > 0);                     // a real rejection of an under-margined entrant
    }

    /// PROOF (FUNDING SCHEDULE) — rate = clamp(base + slope*imbalance, 0, max): monotone
    /// NON-DECREASING in imbalance, floored at base, capped at max; the (rate, direction) it
    /// produces is NEVER rejected by funding_transfer (sign always opposes imbalance).
    #[kani::proof]
    fn proof_funding_schedule_clamped_and_accepted() {
        let imb_a: u128 = kani::any();
        kani::assume(imb_a <= SCALE);
        let imb_b: u128 = kani::any();
        kani::assume(imb_b <= SCALE);
        let base: u128 = kani::any();
        kani::assume(base <= 10_000);
        let slope: u128 = kani::any();
        kani::assume(slope <= 10_000);
        let max: u128 = kani::any();
        kani::assume(base <= max && max <= 10_000); // engine hard max 10_000

        let r_a = funding_rate_from_imbalance(imb_a, base, slope, max);
        let r_b = funding_rate_from_imbalance(imb_b, base, slope, max);
        assert!(r_a >= core::cmp::min(base, max)); // floored at base (base<=max so == base)
        assert!(r_a <= max);                       // capped at max (never exceeds engine max)
        if imb_a <= imb_b {
            assert!(r_a <= r_b);                   // monotone non-decreasing in imbalance
        }

        // The directed decision is ALWAYS accepted by funding_transfer (sign opposes imbalance).
        let m_long: u128 = kani::any();
        kani::assume(m_long >= 1 && m_long <= 1_000_000_000);
        let m_short: u128 = kani::any();
        kani::assume(m_short >= 1 && m_short <= 1_000_000_000);
        let (rate, dir) = funding_decision(m_long, m_short, base, slope, max);
        assert!(funding_transfer(m_long, m_short, rate, dir).is_some()); // never wrong-signed

        kani::cover!(imb_a == 0 && r_a == base);                 // floor (imbalance 0 -> base)
        kani::cover!(imb_a > 0 && r_a > base && r_a < max);      // linear region
        kani::cover!(r_a == max && base < max);                  // saturation (cap binds)
        kani::cover!(imb_a < imb_b && r_a < r_b);                // strictly increasing region
    }

    /// PROOF (FEE ROUTING) — split conserves the fee; Residual (fee_to_insurance_bps==0) routes
    /// 100% to the seed; the split is monotone in fee_to_insurance_bps.
    #[kani::proof]
    fn proof_fee_split_conserves_and_routes_to_seed() {
        let fee: u128 = kani::any();
        kani::assume(fee <= 1_000_000_000);
        let bps_a: u128 = kani::any();
        kani::assume(bps_a <= 20_000); // allow >BPS to exercise the clamp
        let bps_b: u128 = kani::any();
        kani::assume(bps_b <= 20_000);

        let (ins_a, seed_a) = split_insurance_fee(fee, bps_a);
        // CONSERVATION: nothing created or lost.
        assert!(ins_a + seed_a == fee);
        assert!(ins_a <= fee);
        // RESIDUAL CASE: 100% to seed.
        if bps_a == 0 {
            assert!(ins_a == 0 && seed_a == fee);
        }
        // MONOTONE in fee_to_insurance_bps (more bps -> >= insurance share).
        if bps_a <= bps_b {
            let (ins_b, _) = split_insurance_fee(fee, bps_b);
            assert!(ins_a <= ins_b);
        }

        kani::cover!(bps_a == 0 && fee > 0 && seed_a == fee); // 100%-to-seed actually witnessed
        kani::cover!(ins_a > 0 && seed_a > 0);                // a real split occurs
        kani::cover!(bps_a >= BPS && ins_a == fee);           // clamp at BPS -> all to insurance
    }

    /// PROOF (LOCKSTEP CLOSE) — both legs reduce by the SAME notional (OI parity preserved),
    /// total collateral conserved across the close, the losing leg never overpays.
    #[kani::proof]
    fn proof_lockstep_close_parity_and_conservation() {
        let m_long: u128 = kani::any();
        kani::assume(m_long <= 1_000_000_000);
        let m_short: u128 = kani::any();
        kani::assume(m_short <= 1_000_000_000);
        let oi: u128 = kani::any();
        kani::assume(oi <= 1_000_000_000);
        let n_close: u128 = kani::any();
        kani::assume(n_close <= 2_000_000_000);
        let pnl: u128 = kani::any();
        kani::assume(pnl <= 100_000); // up to 1000% realized move
        let up: bool = kani::any();

        let before = m_long.saturating_add(m_short);
        let c = lockstep_close(m_long, m_short, oi, n_close, pnl, up);
        // OI PARITY: both legs end at the same OI.
        assert!(c.oi_long_after == c.oi_short_after);
        assert!(c.oi_long_after <= oi); // OI only shrinks
        // CONSERVATION: collateral only redistributed.
        assert!(c.new_long.saturating_add(c.new_short) == before);
        // SEED/LOSER LEG never overpays: draw <= the losing leg's pool.
        let m_l = if up { m_short } else { m_long };
        assert!(c.draw <= m_l);

        kani::cover!(c.draw > 0 && oi > 0 && n_close > 0);       // a real lockstep close moved value
        kani::cover!(c.oi_long_after == 0 && oi > 0);            // full wind-down reachable
        kani::cover!(c.draw == m_l && m_l > 0);                  // loser leg fully drained (cap binds)
    }

    /// PROOF (SEED ACCOUNT) — fillable depth is monotone NON-INCREASING as realized_loss accrues,
    /// and is 0 exactly when realized_loss == posted; reclaimable is never negative and equals
    /// posted once at_risk == 0; book_loss never pushes realized_loss past posted.
    #[kani::proof]
    fn proof_seed_account_winddown() {
        let posted: u128 = kani::any();
        kani::assume(posted <= 1_000_000_000);
        let rl_a: u128 = kani::any();
        kani::assume(rl_a <= posted);
        let rl_b: u128 = kani::any();
        kani::assume(rl_b <= posted);
        let at_risk: u128 = kani::any();
        kani::assume(at_risk <= posted);
        let oracle: u128 = kani::any();
        kani::assume(oracle >= 1 && oracle <= 1_000_000_000);

        let acc_a = SeedAccount { posted, realized_loss: rl_a, at_risk };
        let acc_b = SeedAccount { posted, realized_loss: rl_b, at_risk };
        // MONOTONE NON-INCREASING: more realized loss -> not-larger fillable depth.
        if rl_a <= rl_b {
            assert!(acc_a.fillable_depth(oracle) >= acc_b.fillable_depth(oracle));
        }
        // Exhausted seed -> zero depth.
        if rl_a == posted {
            assert!(acc_a.fillable_depth(oracle) == 0);
        }
        // RECLAIMABLE never negative; equals posted when no OI is at risk.
        assert!(acc_a.reclaimable() <= posted);
        if at_risk == 0 {
            assert!(acc_a.reclaimable() == posted);
        }
        // book_loss never overshoots posted.
        let mut acc_c = SeedAccount::new(posted);
        acc_c.realized_loss = rl_a;
        let extra: u128 = kani::any();
        kani::assume(extra <= 4_000_000_000);
        let booked = acc_c.book_loss(extra);
        assert!(acc_c.realized_loss <= posted);
        assert!(booked <= extra);

        kani::cover!(rl_a < rl_b && acc_a.fillable_depth(oracle) > acc_b.fillable_depth(oracle)); // strict drop
        kani::cover!(rl_a == posted && posted > 0);   // exhausted seed reachable
        kani::cover!(at_risk == 0 && posted > 0);     // clean reclaim reachable
        kani::cover!(booked < extra);                 // book_loss cap actually binds
    }

    /// PROOF (PER-DOMAIN ISOLATION) — each market's realizable draw is bounded by ITS OWN budget
    /// (== its seed), never a shared term; two markets' loss bounds add disjointly and a 0-budget
    /// market cannot subsidize another.
    #[kani::proof]
    fn proof_domain_budget_disjoint() {
        let seed1: u128 = kani::any();
        kani::assume(seed1 <= 1_000_000_000);
        let seed2: u128 = kani::any();
        kani::assume(seed2 <= 1_000_000_000);
        let claim1: u128 = kani::any();
        kani::assume(claim1 <= 4_000_000_000);
        let claim2: u128 = kani::any();
        kani::assume(claim2 <= 4_000_000_000);

        let b1 = domain_budget_at_activation(seed1);
        let b2 = domain_budget_at_activation(seed2);
        assert!(b1 == seed1 && b2 == seed2); // budget == own seed, no global term

        let d1 = realizable_draw_in_domain(claim1, b1);
        let d2 = realizable_draw_in_domain(claim2, b2);
        // Each draw bounded by its OWN budget; combined loss bounded by the SUM (disjoint).
        assert!(d1 <= seed1);
        assert!(d2 <= seed2);
        assert!(d1.saturating_add(d2) <= seed1.saturating_add(seed2));
        // A 0-budget market draws nothing -> cannot subsidize the other.
        if seed2 == 0 {
            assert!(d2 == 0);
        }

        kani::cover!(d1 == seed1 && seed1 > 0 && d2 < seed2); // market 1 exhausted, market 2 not
        kani::cover!(seed2 == 0 && claim2 > 0 && d2 == 0);    // 0-budget market pays nothing
    }
}

// =============================================================================
// STRESS HARNESS (cargo test). Large sweeps + worked examples, with non-vacuity asserts.
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;

    /// isqrt sanity: floor property holds across a sweep.
    #[test]
    fn isqrt_is_floor_sqrt() {
        for x in [0u128, 1, 2, 3, 4, 5, 8, 9, 15, 16, 17, 99, 100, 101, 1_000_000, 1_000_003] {
            let s = isqrt(x);
            assert!(s.saturating_mul(s) <= x, "isqrt({x})={s} too big");
            assert!((s + 1).saturating_mul(s + 1) > x, "isqrt({x})={s} too small");
        }
        // big values
        for x in [10_000_000_000u128, 123_456_789_012, u64::MAX as u128] {
            let s = isqrt(x);
            assert!(s.saturating_mul(s) <= x);
            assert!((s + 1).saturating_mul(s + 1) > x);
        }
    }

    /// THE BIG SWEEP: across seeds x delta_max x leverage x moves, the seed NEVER loses more than
    /// the seed. >10k combinations. Non-vacuity: saw the cap bind (loss==seed) AND partial loss.
    #[test]
    fn stress_seed_never_loses_more_than_seed() {
        let seeds = [1_000u128, 5_000, 10_000, 100_000, 1_000_000, 10_000_000, 100_000_000, 500_000_000];
        let deltas = [0u128, 100, 250, 500, 1_250, 2_500, 5_000, 10_000, 20_000, 50_000, 100_000];
        // "leverage" enters via how big the crowd's notional is relative to the cap; we sweep a
        // notional-multiplier that can exceed the cap (the matcher would clip, but we also test
        // the raw bound holds even if it didn't).
        let lev_mult = [1u128, 2, 3, 5, 8, 10, 20, 50, 100]; // crowd tries lev_mult * n_cap notional
        let moves = [0u128, 10, 25, 50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 25_000, 50_000, 100_000];
        let (mut saw_full, mut saw_cap_binds, mut saw_wipe) = (false, false, false);
        let mut combos = 0u64;
        for &seed in &seeds {
            for &delta in &deltas {
                let n_cap = oi_cap_sized_to_seed(seed, delta);
                assert!(n_cap <= seed, "cap exceeds seed: seed={seed} delta={delta} cap={n_cap}");
                for &lm in &lev_mult {
                    // Even if the crowd's notional is lm*n_cap, the MATCHER clips at n_cap; so the
                    // seed-facing notional is min(lm*n_cap, n_cap) = n_cap. We assert the realized
                    // loss bound on the capped (real) path AND the RAW-CLAIM cap-binding theorem:
                    // the clip is load-bearing precisely when an UNCLIPPED notional would have
                    // produced a raw claim exceeding the seed while the CAPPED raw claim does not.
                    let capped_notional = n_cap; // matcher clip
                    let raw_notional = lm.saturating_mul(n_cap); // pretend no clip
                    for &r in &moves {
                        let loss_capped = seed_loss_on_position(seed, capped_notional, r);
                        let loss_raw = seed_loss_on_position(seed, raw_notional, r);
                        assert!(loss_capped <= seed, "CAPPED BLEED seed={seed} cap={n_cap} r={r}");
                        assert!(loss_raw <= seed, "RAW BLEED seed={seed} n={raw_notional} r={r}");
                        // Raw claims: the CAPPED claim is what the matcher actually exposes the
                        // seed to; the UNCLIPPED claim is what a missing cap would expose.
                        let capped_claim = raw_claim_on_seed(capped_notional, r);
                        let unclipped_claim = raw_claim_on_seed(raw_notional, r);
                        if capped_claim > 0 && capped_claim <= seed {
                            saw_full = true; // capped claim within the seed -> winner paid in full
                        }
                        // CAP IS LOAD-BEARING: the unclipped claim would have BLOWN PAST the seed
                        // (raw claim > seed) but the matcher clip keeps the seed-facing raw claim
                        // within the seed. This witnesses the clip preventing a bleed for real.
                        if unclipped_claim > seed && capped_claim <= seed {
                            saw_cap_binds = true;
                        }
                        if loss_raw == seed && seed > 0 {
                            saw_wipe = true; // the seed was fully consumed (unclipped path)
                        }
                        combos += 1;
                    }
                }
            }
        }
        assert!(combos > 10_000, "sweep too small: {combos}");
        assert!(saw_full, "VACUOUS: never saw a fully-seed-covered claim");
        assert!(saw_cap_binds, "VACUOUS: never saw the cap prevent a raw claim from exceeding the seed");
        assert!(saw_wipe, "VACUOUS: never saw the seed fully consumed");
    }

    /// BREAK-EVEN WORKED EXAMPLES (spec section 8): reproduce BOTH spec cases USING SPEC-BAND
    /// funding rates (clamp(100, 3000) e9/slot) — the precision fix lets a base-band rate matter.
    ///   * a ~6h fast pump where the seed BLEEDS (income < adverse), and
    ///   * a ~7-day persistent run where it EARNS (income > adverse).
    /// Both the SIGN and the EXACT income/adverse formulas are pinned (catches a 0.8-coefficient
    /// or spread-drop regression that a pure sign check on a wide margin would miss).
    #[test]
    fn breakeven_worked_examples() {
        // Slot model: ~2.5 slots/sec on Solana -> ~9000 slots/hr. 6h ~= 54_000 slots;
        // 7 days ~= 1_512_000 slots. Params from the spec's starting set:
        //   fee=30bps, spread=20bps. sigma per slot ~ a couple bps.
        let fee = 30u128;
        let spread = 20u128;

        // CASE 1 — 6h FAST PUMP: high-ish sigma, BASE funding (a sudden pump has not yet built a
        // sustained imbalance, so funding sits at the schedule floor base=100 e9/slot). Adverse
        // selection dominates over the short window -> BLEEDS.
        let tau_pump = 54_000u128; // ~6h
        let sigma_pump = 5u128; // 5 bps/slot
        let funding_pump_e9 = 100u128; // spec schedule FLOOR (imbalance ~0)
        let r1 = paid_lp_pnl(fee, spread, funding_pump_e9, tau_pump, sigma_pump, 1_000_000);
        assert!(!r1.net_positive, "6h fast pump should BLEED: income={} adverse={}", r1.income, r1.adverse);
        let saw_bleed = !r1.net_positive;

        // EXACT FORMULA PIN (catches drop-spread / wrong-divide-order regressions):
        assert_eq!(r1.income, LpPnl::closed_form_income(fee, spread, funding_pump_e9, tau_pump));
        assert_eq!(r1.adverse, LpPnl::closed_form_adverse(sigma_pump, tau_pump));
        // INDEPENDENT NUMERIC BAND that PINS the 0.8 coefficient (computed by hand, NOT from the
        // function under test): sigma=5, tau=54_000 -> isqrt=232 -> 5e6*232*4/50000 = 92_800.
        // A 0.2 coeff would give 23_200; a no-coefficient (1.0) term gives 116_000 — the tight band
        // [85_000, 100_000] catches BOTH a shrunk and an inflated coefficient.
        assert!(isqrt(tau_pump) == 232, "isqrt(54000) sanity for the hand-computed band");
        assert!(r1.adverse >= 85_000 && r1.adverse <= 100_000,
            "adverse {} outside the 0.8-coefficient band [85000,100000]", r1.adverse);
        assert_eq!(r1.adverse, 92_800, "adverse must equal the hand-computed 0.8-coefficient value");
        // And the spread is load-bearing in income: fee-only income is strictly smaller.
        let income_fee_only = LpPnl::closed_form_income(fee, 0, funding_pump_e9, tau_pump);
        assert!(r1.income > income_fee_only, "spread must contribute to income");
        // Pin income too: turnover (50bps -> 5000) + funding (100*54000*1e6/1e9 = 5400) = 10_400.
        assert_eq!(r1.income, 10_400, "income must equal the hand-computed turnover+funding value");

        // CASE 2 — 7-DAY PERSISTENT RUN: a sustained one-sided imbalance pays funding the WHOLE
        // time at (near) the schedule CEILING (3000 e9/slot); turnover accrues; sqrt(tau) grows
        // slower than tau so funding income wins on TIME -> EARNS.
        let tau_run = 1_512_000u128; // ~7 days
        let sigma_run = 5u128; // same per-slot volatility
        let funding_run_e9 = 3_000u128; // spec schedule CEILING (sustained one-sided imbalance)
        let r2 = paid_lp_pnl(fee, spread, funding_run_e9, tau_run, sigma_run, 1_000_000);
        assert!(r2.net_positive, "7-day persistent run should EARN: income={} adverse={}", r2.income, r2.adverse);
        let saw_paid = r2.net_positive;

        assert_eq!(r2.income, LpPnl::closed_form_income(fee, spread, funding_run_e9, tau_run));
        assert_eq!(r2.adverse, LpPnl::closed_form_adverse(sigma_run, tau_run));

        // BOTH directions must actually occur (non-vacuity).
        assert!(saw_bleed && saw_paid, "must observe BOTH a bleed and a paid case");

        // The funding income is NON-ZERO at the spec base rate (precision-fix witness: the old
        // per-slot truncation made base-rate funding income exactly 0).
        let funding_only_pump = LpPnl::closed_form_income(0, 0, funding_pump_e9, tau_pump);
        assert!(funding_only_pump > 0, "spec base-rate funding must contribute (precision fix)");

        // Sanity on directions vs the spec's regimes.
        assert!(r1.adverse > r1.income); // pump: adverse dominates
        assert!(r2.income > r2.adverse); // run: funding-on-time wins
    }

    /// Break-even MONOTONICITY: holding sigma+funding fixed, longer tau eventually flips a bleed
    /// into a paid position (funding ~ tau beats adverse ~ sqrt(tau)). Demonstrates the spec's
    /// "funding wins on TIME". Sweeps tau and finds the crossover, asserting it exists.
    #[test]
    fn breakeven_funding_wins_on_time() {
        // Thin turnover (fee+spread=10bps) and per-slot volatility (sigma=10bps) so adverse
        // selection dominates at short tau; the SPEC-BAND CEILING funding rate (3000 e9/slot, the
        // top of clamp(100,3000)) accrues per slot and overtakes adverse (~tau beats ~sqrt(tau))
        // at long tau. Crossover lands between tau=50k and tau=100k.
        let (fee, spread, sigma) = (5u128, 5u128, 10u128);
        let funding_e9 = 3_000u128; // spec-band sustained funding CEILING (within [100,3000])
        let mut saw_bleed = false;
        let mut saw_paid = false;
        let mut crossed = false;
        let mut prev_paid = false;
        let mut saw_funding_contrib = false;
        for &tau in &[1_000u128, 10_000, 50_000, 100_000, 300_000, 600_000, 1_000_000, 2_000_000, 5_000_000] {
            let r = paid_lp_pnl(fee, spread, funding_e9, tau, sigma, 1_000_000);
            // funding income must be non-zero at this spec-band rate (precision-fix witness).
            let funding_only = LpPnl::closed_form_income(0, 0, funding_e9, tau);
            if funding_only > 0 {
                saw_funding_contrib = true;
            }
            if r.net_positive {
                saw_paid = true;
            } else {
                saw_bleed = true;
            }
            if r.net_positive && !prev_paid {
                crossed = true; // bleed -> paid crossover as tau grows
            }
            prev_paid = r.net_positive;
        }
        assert!(saw_bleed, "VACUOUS: never bled at short tau");
        assert!(saw_paid, "VACUOUS: never got paid at long tau");
        assert!(crossed, "funding should overtake adverse as time grows");
        assert!(saw_funding_contrib, "VACUOUS: spec-band funding never contributed (precision bug?)");
    }

    /// SEED EXHAUSTION LIFECYCLE: drive a one-sided book to seed exhaustion. Assert credit_rate
    /// falls to a floor, total loss == seed (not more), conservation holds each step.
    #[test]
    fn seed_exhaustion_lifecycle() {
        let seed = 1_000_000u128;
        // Each winning claim is a chunk of the seed; drive many steps until exhausted.
        let claim_per_step = 250_000u128; // 1/4 of seed per win
        // Manual step-through to check per-step conservation (winner_out + remaining == prior).
        let mut backing = seed;
        let mut paid_total = 0u128;
        let mut prev_cr = SCALE;
        let mut saw_haircut = false;
        let mut saw_wipe = false;
        for _ in 0..10 {
            let prior = backing;
            let cr = credit_rate(backing, claim_per_step);
            assert!(cr <= prev_cr, "credit_rate must not increase as backing drains");
            let (out, nb) = drain_step(backing, claim_per_step);
            // EXACT VALUE: out is the credit-rate-discounted claim (not min(claim, backing)).
            assert_eq!(out, claim_per_step.saturating_mul(cr) / SCALE, "out must equal claim*cr/SCALE");
            // CONSERVATION per step: what the winner took + what remains == what was there.
            assert_eq!(out + nb, prior, "per-step conservation");
            // never overpaid, never insolvent.
            assert!(out <= claim_per_step);
            assert!(out <= prior);
            if cr < SCALE {
                saw_haircut = true;
            }
            if nb == 0 {
                saw_wipe = true;
            }
            paid_total = paid_total.saturating_add(out);
            backing = nb;
            prev_cr = cr;
        }
        // The seed loses EXACTLY its whole backing (not more), and that's all that was paid out.
        assert_eq!(paid_total, seed, "total paid to winners == seed (loss == seed, not more)");
        assert_eq!(backing, 0, "seed fully exhausted");
        assert!(saw_haircut, "VACUOUS: never saw a haircut as the seed drained");
        assert!(saw_wipe, "VACUOUS: never saw the seed hit zero");

        // Cross-check via drain_sequence helper + its monotonicity flag.
        let outcome = drain_sequence(seed, claim_per_step, 10);
        assert!(outcome.monotone, "credit_rate trajectory must be monotone non-increasing");
        assert_eq!(outcome.total_out, seed);
        assert_eq!(outcome.final_backing, 0);
        assert_eq!(outcome.min_credit_rate, 0, "credit_rate floor reached (0 once backing==0)");

        // MID-POSITION PARTIAL HAIRCUT (AF): a claim that does NOT divide the seed lands a step
        // with 0 < credit_rate < 1 — the gradual draining-haircut the spec section 10 calls out
        // (NOT the 1,1,1,0 cliff above). The winner takes a PROPORTIONAL haircut (out < claim),
        // while the seed's total loss stays capped at the seed.
        let seed2 = 1_000_000u128;
        let claim2 = 300_000u128; // does NOT divide 1_000_000 -> a fractional-credit_rate step lands
        let mut backing2 = seed2;
        let mut paid2 = 0u128;
        let mut saw_partial_haircut = false; // 0 < cr < 1 seen on a NON-exhausted step
        for _ in 0..10 {
            if backing2 == 0 {
                break;
            }
            let cr = credit_rate(backing2, claim2);
            let (out, nb) = drain_step(backing2, claim2);
            assert_eq!(out, claim2.saturating_mul(cr) / SCALE);
            // When underfunded, the winner is PROPORTIONALLY haircut: out < claim, out <= backing
            // (out is backing up to integer-rounding floor: backing - out < claim2/SCALE + 1).
            if cr > 0 && cr < SCALE {
                saw_partial_haircut = true;
                assert!(out < claim2, "underfunded step must haircut the winner");
                assert!(out <= backing2, "a partial-fill step cannot overpay the remaining backing");
                // out is essentially the whole remaining backing (within one rounding unit of it).
                assert!(backing2 - out < SCALE, "haircut pays out (nearly) all remaining backing");
            }
            paid2 = paid2.saturating_add(out);
            backing2 = nb;
        }
        assert!(saw_partial_haircut, "VACUOUS: never saw a mid-position 0<cr<1 partial haircut");
        assert!(paid2 <= seed2, "mid-position lifecycle never paid more than the seed");
    }

    /// SEED EXHAUSTION sweep: many (seed, claim) shapes; for ALL of them total_out <= seed,
    /// monotone holds, and final credit_rate <= initial. Non-vacuity across shapes.
    #[test]
    fn seed_exhaustion_sweep() {
        let seeds = [10_000u128, 100_000, 1_000_000, 50_000_000];
        let claims = [1u128, 1_000, 33_333, 250_000, 1_000_000, 9_999_999];
        let mut saw_partial = false;
        let mut saw_exhaust = false;
        let mut combos = 0u64;
        for &seed in &seeds {
            for &claim in &claims {
                let o = drain_sequence(seed, claim, 64);
                assert!(o.total_out <= seed, "drained more than the seed seed={seed} claim={claim}");
                assert!(o.monotone, "non-monotone credit_rate seed={seed} claim={claim}");
                assert!(o.final_credit_rate <= SCALE);
                if o.final_backing > 0 {
                    saw_partial = true;
                }
                if o.final_backing == 0 {
                    saw_exhaust = true;
                }
                combos += 1;
            }
        }
        assert!(combos >= 24);
        assert!(saw_partial, "VACUOUS: every run exhausted the seed");
        assert!(saw_exhaust, "VACUOUS: no run ever exhausted the seed");
    }

    /// FUNDING PULLS + SIGN sweep: across imbalances and rates, funding ALWAYS conserves total
    /// and its sign opposes the imbalance; a wrong-sign request is rejected.
    #[test]
    fn funding_pulls_and_sign() {
        let pools = [
            (0u128, 0u128),
            (500, 500),
            (900, 100),
            (100, 900),
            (1_000_000, 1),
            (1, 1_000_000),
            (123_456, 654_321),
        ];
        // Rates RAISED past E9 (2x, 5x) so the payer-cap actually fires (was dead code before).
        let rates = [0u128, 1, 1_000, 100_000, 10_000_000, 500_000_000, E9, 2 * E9, 5 * E9];
        let mut saw_long_pays = false;
        let mut saw_short_pays = false;
        let mut saw_reject = false;
        let mut saw_move = false;
        let mut saw_cap_binds = false; // payer-cap fired (amount == whole payer pool)
        let mut combos = 0u64;
        for &(ml, ms) in &pools {
            let before = ml + ms;
            for &rate in &rates {
                for &want in &[true, false] {
                    match funding_transfer(ml, ms, rate, want) {
                        Some(t) => {
                            assert_eq!(t.new_long + t.new_short, before, "funding must conserve total");
                            if t.amount > 0 {
                                saw_move = true;
                                // sign opposes imbalance + EXACT magnitude tied to the PAYER pool.
                                let payer = if t.long_pays { ml } else { ms };
                                let expected = core::cmp::min(payer.saturating_mul(rate) / E9, payer);
                                assert_eq!(t.amount, expected, "amount must be payer-capped schedule");
                                assert!(t.amount <= payer, "moved more than the payer holds");
                                if t.amount == payer && payer > 0 {
                                    saw_cap_binds = true; // payer-cap actually bound
                                }
                                if t.long_pays {
                                    assert!(ml > ms, "long paid but long not crowded");
                                    saw_long_pays = true;
                                } else {
                                    assert!(ms > ml, "short paid but short not crowded");
                                    saw_short_pays = true;
                                }
                            }
                        }
                        None => {
                            // rejected: must be an imbalanced book with a wrong-sign request.
                            assert_ne!(ml, ms);
                            let correct = ml > ms;
                            assert_ne!(want, correct);
                            saw_reject = true;
                        }
                    }
                    combos += 1;
                }
            }
        }
        assert!(combos > 50);
        assert!(saw_long_pays, "VACUOUS: never saw long pay short");
        assert!(saw_short_pays, "VACUOUS: never saw short pay long");
        assert!(saw_reject, "VACUOUS: never saw a wrong-sign rejection");
        assert!(saw_move, "VACUOUS: funding never actually moved value");
        assert!(saw_cap_binds, "VACUOUS: the payer-cap never bound (rate>E9 path untested)");
    }

    /// THIN-REBATE sweep: surcharge XOR rebate (never both), total_bps >= 0 (never crosses
    /// oracle), and the FIXED is_thin condition behaves per spec across the inventory/side grid.
    #[test]
    fn thin_rebate_sign_and_no_cross() {
        let invs = [0u128, 1, 100, 5_000, 1_000_000];
        let mults = [0u128, 1, 50, 500, 5_000, 50_000];
        let spreads = [0u128, 5, 20, 100, 1_000];
        let mut saw_rebate = false;
        let mut saw_surcharge = false;
        let mut saw_zeroed = false; // rebate fully consumed the spread (total==0, not negative)
        let mut combos = 0u64;
        for &inv in &invs {
            for &is_long in &[true, false] {
                for &is_buy in &[true, false] {
                    for &mult in &mults {
                        for &spread in &spreads {
                            let a = thin_rebate_bps(inv, is_long, is_buy, mult, spread);
                            // XOR: never both.
                            assert!(!(a.surcharge_bps > 0 && a.rebate_bps > 0), "both surcharge and rebate!");
                            // never crosses oracle: total >= 0 (u128) AND rebate capped at spread.
                            assert!(a.rebate_bps <= spread);
                            // is_thin matches the spec formula.
                            let inv_nonneg = is_long || inv == 0;
                            let inv_nonpos = !is_long || inv == 0;
                            let expect_thin = (is_buy && inv_nonneg) || (!is_buy && inv_nonpos);
                            assert_eq!(a.is_thin, expect_thin, "is_thin formula mismatch");
                            // EXACT SLOPE (recomputed FROM INPUTS): catches a doubled/halved A11 slope.
                            let uncapped = inv.saturating_mul(mult) / BPS;
                            if a.is_thin {
                                assert_eq!(a.surcharge_bps, 0);
                                assert_eq!(a.rebate_bps, core::cmp::min(uncapped, spread), "rebate slope");
                                assert_eq!(a.total_bps, spread - a.rebate_bps);
                                if a.rebate_bps > 0 {
                                    saw_rebate = true;
                                }
                                if a.total_bps == 0 && spread > 0 {
                                    saw_zeroed = true;
                                }
                            } else {
                                assert_eq!(a.rebate_bps, 0);
                                assert_eq!(a.surcharge_bps, uncapped, "surcharge slope (uncapped)");
                                assert_eq!(a.total_bps, spread.saturating_add(a.surcharge_bps));
                                if a.surcharge_bps > 0 {
                                    saw_surcharge = true;
                                }
                            }
                            combos += 1;
                        }
                    }
                }
            }
        }
        assert!(combos > 100);
        assert!(saw_rebate, "VACUOUS: never observed a real rebate");
        assert!(saw_surcharge, "VACUOUS: never observed a real surcharge");
        assert!(saw_zeroed, "VACUOUS: never saw a rebate consume the whole spread (total==0)");
    }

    /// DEPTH CLIP sweep: clip <= opposing depth; pool non-increasing; zero-fill on empty pool;
    /// no panic anywhere.
    #[test]
    fn depth_clip_safe_sweep() {
        let opps = [0u128, 1, 1_000, 1_000_000, 1_000_000_000];
        let oracles = [0u128, 1, 1_000_000, 50_000_000, 2_000_000_000];
        let maxinvs = [0u128, 1, 100, 1_000_000, 1_000_000_000_000];
        let fills = [0u128, 1, 50, 1_000_000, 1_000_000_000_000];
        let mut saw_zero_empty = false;
        let mut saw_cap_binds = false;
        let mut saw_full_fill = false;
        let mut combos = 0u64;
        for &opp in &opps {
            for &oracle in &oracles {
                for &maxinv in &maxinvs {
                    for &fill in &fills {
                        let clip = residual_depth_clip(opp, oracle, maxinv, fill);
                        assert!(clip <= fill);
                        assert!(clip <= maxinv);
                        if opp == 0 || oracle == 0 {
                            assert_eq!(clip, 0, "empty pool / no price must zero-fill");
                            if opp == 0 {
                                saw_zero_empty = true;
                            }
                        } else {
                            let depth_q = opp.saturating_mul(SCALE) / oracle;
                            assert!(clip <= depth_q);
                            // pool non-increasing after deduction:
                            let new_pool = deduct_pool_at_oracle(opp, clip, oracle);
                            assert!(new_pool <= opp, "pool grew!");
                        }
                        if clip > 0 && clip == maxinv {
                            saw_cap_binds = true;
                        }
                        if clip > 0 && clip == fill {
                            saw_full_fill = true;
                        }
                        combos += 1;
                    }
                }
            }
        }
        assert!(combos > 100);
        assert!(saw_zero_empty, "VACUOUS: never saw the empty-pool zero-fill");
        assert!(saw_cap_binds, "VACUOUS: OI cap never bound the clip");
        assert!(saw_full_fill, "VACUOUS: a request was never fully filled");
    }

    /// OI-CAP WORKED EXAMPLE (spec section 6): Seed=$10k, move=25bps/slot, dt=50 -> Delta_max=1250
    /// -> oracle-gate=$80k, binding cap = min(10k, 80k) = $10k (the 1:1 bound binds). Also the
    /// conservative half-seed variant and a volatile market where the oracle-gate binds.
    #[test]
    fn oi_cap_worked_example() {
        let seed = 10_000u128; // $10k in whole-dollar atoms
        let dmax = delta_max_bps(25, 50); // 1250 bps
        assert_eq!(dmax, 1_250);
        let oracle_gate = seed * BPS / dmax; // 10000 * 10000 / 1250 = 80_000
        assert_eq!(oracle_gate, 80_000);
        let cap = oi_cap_sized_to_seed(seed, dmax);
        assert_eq!(cap, seed, "1:1 seed bound is the tighter (binding) side -> cap == $10k");

        // max_inventory_abs at oracle price $50.000000 (50e6):
        let oracle_e6 = 50_000_000u128;
        let mia = max_inventory_abs_from_cap(cap, oracle_e6);
        assert_eq!(mia, cap * SCALE / oracle_e6, "max_inventory_abs = N_cap*1e6/oracle");

        // A VOLATILE market: move 500bps/slot, dt 50 -> Delta_max=25_000 -> oracle-gate = $4k <
        // seed -> oracle-gate BINDS (smaller cap for the riskier market).
        let dmax2 = delta_max_bps(500, 50); // 25_000
        let cap2 = oi_cap_sized_to_seed(seed, dmax2);
        assert!(cap2 < seed, "volatile market -> oracle-gate is the binding (tighter) side");
        assert_eq!(cap2, seed * BPS / dmax2); // 10000*10000/25000 = 4_000

        // oracle==0 -> no inventory allowed (closes the silent-unlimited loophole).
        assert_eq!(max_inventory_abs_from_cap(cap, 0), 0);
    }

    /// A buggy thin-rebate (the ORIGINAL spec bug: the WORSENS branch as a rebate) would make
    /// total_bps cross the oracle (go below 0 / overflow on surcharge-as-rebate). This test pins
    /// that our FIXED is_thin matches the documented complement-of-skew condition for the two
    /// canonical cases (residual buy with seed long -> thin; residual sell with seed long ->
    /// worsens), so a regression to the buggy sign is caught.
    #[test]
    fn thin_rebate_fixed_sign_canonical() {
        let spread = 20u128;
        let mult = 100u128;
        let inv = 10_000u128;
        // seed net long (inv>=0):
        // is_buy -> THIN (rebate). !is_buy -> WORSENS (surcharge).
        let buy = thin_rebate_bps(inv, true, true, mult, spread);
        assert!(buy.is_thin && buy.rebate_bps > 0 && buy.surcharge_bps == 0);
        assert!(buy.total_bps <= spread, "rebate must not raise the spread");
        let sell = thin_rebate_bps(inv, true, false, mult, spread);
        assert!(!sell.is_thin && sell.surcharge_bps > 0 && sell.rebate_bps == 0);
        assert!(sell.total_bps >= spread, "surcharge must raise the spread");
        // seed net short (inv<=0): mirror image.
        let buy_s = thin_rebate_bps(inv, false, true, mult, spread);
        assert!(!buy_s.is_thin, "seed short + buy worsens");
        let sell_s = thin_rebate_bps(inv, false, false, mult, spread);
        assert!(sell_s.is_thin, "seed short + sell thins");
    }

    /// LEVERAGE RATCHET sweep (coverage gap #1): initial_margin_bps is monotone non-decreasing in
    /// crowded OI, floored at 10x (1000bps), capped at 1x (BPS); the gate rejects an under-margined
    /// late entrant; at/over the cap full margin is required.
    #[test]
    fn leverage_ratchet_gate() {
        let n_caps = [1_000u128, 10_000, 1_000_000];
        let ois = [0u128, 1, 100, 500, 1_000, 5_000, 10_000, 1_000_000, 2_000_000];
        let mut saw_floor = false;
        let mut saw_cap = false;
        let mut saw_mid = false;
        let mut saw_reject = false;
        let mut combos = 0u64;
        for &n_cap in &n_caps {
            let mut prev = 0u128;
            for (i, &oi) in ois.iter().enumerate() {
                let m = initial_margin_bps(oi, n_cap);
                assert!(m >= 1000 && m <= BPS, "margin out of [10x,1x] band");
                if i > 0 {
                    // ois is sorted ascending -> monotone non-decreasing.
                    assert!(m >= prev, "ratchet decreased with more crowding n_cap={n_cap} oi={oi}");
                }
                prev = m;
                if m == 1000 && oi < n_cap {
                    saw_floor = true;
                }
                if oi >= n_cap {
                    assert_eq!(m, BPS, "at/over cap must demand 1x margin");
                    saw_cap = true;
                }
                if m > 1000 && m < BPS {
                    saw_mid = true;
                }
                // Gate: a position posting just under the requirement is rejected.
                let notional = 100_000u128;
                let req = notional.saturating_mul(m) / BPS;
                if req > 0 {
                    assert!(!margin_gate_admits(req - 1, notional, oi, n_cap), "under-margin must reject");
                    assert!(margin_gate_admits(req, notional, oi, n_cap), "exact-margin must admit");
                    saw_reject = true;
                }
                combos += 1;
            }
        }
        assert!(combos > 20);
        assert!(saw_floor, "VACUOUS: never saw the 10x floor");
        assert!(saw_cap, "VACUOUS: never saw the 1x cap");
        assert!(saw_mid, "VACUOUS: never saw the mid-ratchet region");
        assert!(saw_reject, "VACUOUS: never exercised a gate rejection");
    }

    /// LEVERAGE RATCHET worked example (spec section 8): max(1000, 1e4*OI/N_cap), 10x empty -> 1x
    /// at the cap. N_cap=$10k: empty -> 1000bps (10x); half-full ($5k) -> 5000bps (2x); full
    /// ($10k) -> BPS (1x); over -> BPS.
    #[test]
    fn leverage_ratchet_worked_example() {
        let n_cap = 10_000u128;
        assert_eq!(initial_margin_bps(0, n_cap), 1000, "empty book -> 10x (1000bps floor)");
        assert_eq!(initial_margin_bps(5_000, n_cap), 5_000, "half-full -> 2x (5000bps)");
        assert_eq!(initial_margin_bps(10_000, n_cap), BPS, "full -> 1x (BPS)");
        assert_eq!(initial_margin_bps(50_000, n_cap), BPS, "over cap -> still 1x");
        // a thin book where the raw ratio is below the floor stays at 10x:
        assert_eq!(initial_margin_bps(100, n_cap), 1000, "thin book clamped to 10x floor");
        // n_cap==0 degenerates to the conservative 1x gate.
        assert_eq!(initial_margin_bps(0, 0), BPS);
    }

    /// FUNDING SCHEDULE sweep (coverage gap #2): clamp(100 + 1500*imbalance, 0, 3000); monotone
    /// in imbalance, floored at base, capped at max; the produced (rate, dir) is always accepted.
    #[test]
    fn funding_schedule_clamp_and_accept() {
        let (base, slope, max) = (100u128, 1500u128, 3000u128);
        // imbalance in SCALE units (0 == balanced, 1e6 == fully one-sided).
        let imbs = [0u128, 100_000, 250_000, 500_000, 750_000, 1_000_000, 2_000_000];
        let mut prev = 0u128;
        let mut saw_floor = false;
        let mut saw_linear = false;
        let mut saw_cap = false;
        for (i, &imb) in imbs.iter().enumerate() {
            let r = funding_rate_from_imbalance(imb, base, slope, max);
            assert!(r >= base && r <= max, "rate out of [base,max] band");
            if i > 0 {
                assert!(r >= prev, "schedule decreased with more imbalance");
            }
            prev = r;
            if imb == 0 {
                assert_eq!(r, base, "balanced -> base rate");
                saw_floor = true;
            }
            if r > base && r < max {
                saw_linear = true;
            }
            if r == max {
                saw_cap = true;
            }
        }
        assert!(saw_floor && saw_linear && saw_cap, "must see floor, linear, and saturation regions");
        // EXACT worked numbers: imbalance 0.5 (500_000 SCALE) -> 100 + 1500*0.5 = 850.
        assert_eq!(funding_rate_from_imbalance(500_000, base, slope, max), 850);
        // imbalance 1.0 -> 100 + 1500 = 1600.
        assert_eq!(funding_rate_from_imbalance(1_000_000, base, slope, max), 1600);

        // The directed decision is always sign-correct -> never rejected by funding_transfer.
        let books = [(900u128, 100u128), (100, 900), (500, 500), (1_000_000, 1)];
        let mut saw_accept_move = false;
        for &(ml, ms) in &books {
            let (rate, dir) = funding_decision(ml, ms, base, slope, max);
            let t = funding_transfer(ml, ms, rate, dir);
            assert!(t.is_some(), "schedule-produced direction must never be rejected");
            if let Some(tt) = t {
                if tt.amount > 0 {
                    saw_accept_move = true;
                    // crowded side pays.
                    if tt.long_pays { assert!(ml > ms); } else { assert!(ms > ml); }
                }
            }
        }
        assert!(saw_accept_move, "VACUOUS: schedule never moved value");
    }

    /// FEE ROUTING sweep (coverage gap #3): split conserves the fee; Residual (0 bps) routes 100%
    /// to the seed; split is monotone in fee_to_insurance_bps.
    #[test]
    fn fee_routing_to_seed() {
        let fees = [0u128, 1, 30, 1_000, 1_000_000];
        let bpss = [0u128, 1, 2_500, 5_000, 10_000, 20_000]; // include >BPS to test the clamp
        let mut saw_all_seed = false;
        let mut saw_split = false;
        let mut saw_clamp = false;
        for &fee in &fees {
            let mut prev_ins = 0u128;
            for (i, &bps) in bpss.iter().enumerate() {
                let (ins, seed) = split_insurance_fee(fee, bps);
                assert_eq!(ins + seed, fee, "fee split must conserve");
                if bps == 0 {
                    assert_eq!(ins, 0);
                    assert_eq!(seed, fee, "Residual: 100% to seed");
                    if fee > 0 {
                        saw_all_seed = true;
                    }
                }
                if i > 0 {
                    assert!(ins >= prev_ins, "insurance share must be monotone in bps");
                }
                prev_ins = ins;
                if ins > 0 && seed > 0 {
                    saw_split = true;
                }
                if bps >= BPS {
                    assert_eq!(ins, fee, "bps>=BPS clamps to 100% insurance");
                    if fee > 0 {
                        saw_clamp = true;
                    }
                }
            }
        }
        assert!(saw_all_seed, "VACUOUS: never saw 100%-to-seed");
        assert!(saw_split, "VACUOUS: never saw a real split");
        assert!(saw_clamp, "VACUOUS: never saw the bps clamp");
        // Worked: fee=1000, 25% insurance -> 250 insurance, 750 seed.
        assert_eq!(split_insurance_fee(1_000, 2_500), (250, 750));
    }

    /// LOCKSTEP CLOSE sweep (coverage gap #4): both legs reduce by the same notional (OI parity);
    /// total collateral conserved; the losing leg never overpays.
    #[test]
    fn lockstep_close_parity_conservation() {
        let mls = [1_000u128, 50_000, 1_000_000];
        let mss = [1_000u128, 50_000, 1_000_000];
        let ois = [0u128, 100, 10_000, 1_000_000];
        let closes = [0u128, 50, 10_000, 2_000_000];
        let pnls = [0u128, 100, 1_000, 50_000];
        let mut saw_move = false;
        let mut saw_full_winddown = false;
        let mut saw_cap_binds = false;
        let mut combos = 0u64;
        for &ml in &mls {
            for &ms in &mss {
                for &oi in &ois {
                    for &nc in &closes {
                        for &pnl in &pnls {
                            for &up in &[true, false] {
                                let before = ml + ms;
                                let c = lockstep_close(ml, ms, oi, nc, pnl, up);
                                assert_eq!(c.oi_long_after, c.oi_short_after, "OI PARITY broken");
                                assert!(c.oi_long_after <= oi);
                                assert_eq!(c.new_long + c.new_short, before, "lockstep close must conserve");
                                let m_l = if up { ms } else { ml };
                                assert!(c.draw <= m_l, "loser leg overpaid");
                                if c.draw > 0 {
                                    saw_move = true;
                                }
                                if oi > 0 && c.oi_long_after == 0 {
                                    saw_full_winddown = true;
                                }
                                if m_l > 0 && c.draw == m_l {
                                    saw_cap_binds = true;
                                }
                                combos += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(combos > 100);
        assert!(saw_move, "VACUOUS: a lockstep close never moved value");
        assert!(saw_full_winddown, "VACUOUS: never fully wound down the OI");
        assert!(saw_cap_binds, "VACUOUS: the loser-leg cap never bound");
    }

    /// SEED ACCOUNT sweep (coverage gap #5): fillable depth monotone non-increasing as realized
    /// loss accrues and 0 exactly at exhaustion; reclaimable never negative, == posted when no OI
    /// is at risk; book_loss never overshoots posted.
    #[test]
    fn seed_account_winddown() {
        let posted = 1_000_000u128;
        let oracle = 50_000_000u128; // $50
        let mut prev_depth = u128::MAX;
        let mut saw_strict_drop = false;
        let mut saw_zero_depth = false;
        for &rl in &[0u128, 100_000, 250_000, 500_000, 999_999, 1_000_000] {
            let acc = SeedAccount { posted, realized_loss: rl, at_risk: 0 };
            let depth = acc.fillable_depth(oracle);
            assert!(depth <= prev_depth, "fillable depth must not increase with realized loss");
            if depth < prev_depth {
                saw_strict_drop = true;
            }
            prev_depth = depth;
            if rl == posted {
                assert_eq!(depth, 0, "exhausted seed -> zero fillable depth");
                saw_zero_depth = true;
            }
        }
        assert!(saw_strict_drop, "VACUOUS: fillable depth never strictly dropped");
        assert!(saw_zero_depth, "VACUOUS: never saw exhausted-seed zero depth");
        // oracle==0 -> zero-fill (not panic).
        assert_eq!(SeedAccount::new(posted).fillable_depth(0), 0);

        // RECLAIMABLE: never negative; == posted when at_risk==0; shrinks with at_risk.
        let mut saw_clean_reclaim = false;
        let mut saw_partial_reclaim = false;
        for &ar in &[0u128, 100_000, 1_000_000] {
            let acc = SeedAccount { posted, realized_loss: 0, at_risk: ar };
            let rec = acc.reclaimable();
            assert!(rec <= posted);
            assert_eq!(rec, posted - ar);
            if ar == 0 {
                assert_eq!(rec, posted, "no OI at risk -> reclaim full posted");
                saw_clean_reclaim = true;
            } else {
                saw_partial_reclaim = true;
            }
        }
        assert!(saw_clean_reclaim && saw_partial_reclaim);

        // book_loss never pushes realized_loss past posted; caps the booked amount.
        let mut acc = SeedAccount::new(posted);
        let b1 = acc.book_loss(400_000);
        assert_eq!(b1, 400_000);
        assert_eq!(acc.realized_loss, 400_000);
        let b2 = acc.book_loss(9_999_999); // would overshoot
        assert_eq!(b2, 600_000, "book_loss caps at remaining room");
        assert_eq!(acc.realized_loss, posted, "realized_loss never exceeds posted");
        assert_eq!(acc.fillable_depth(oracle), 0, "exhausted -> no depth");
    }

    /// PER-DOMAIN ISOLATION sweep (coverage gap #6): each market's draw bounded by its own budget
    /// (== seed); two markets' bounds add disjointly; a 0-budget market subsidizes nobody.
    #[test]
    fn domain_budget_disjoint_isolation() {
        let seeds = [0u128, 1_000, 10_000, 1_000_000];
        let claims = [0u128, 500, 10_000, 5_000_000];
        let mut saw_exhaust_one = false;
        let mut saw_zero_budget = false;
        let mut combos = 0u64;
        for &s1 in &seeds {
            for &s2 in &seeds {
                let b1 = domain_budget_at_activation(s1);
                let b2 = domain_budget_at_activation(s2);
                assert_eq!(b1, s1);
                assert_eq!(b2, s2);
                for &c1 in &claims {
                    for &c2 in &claims {
                        let d1 = realizable_draw_in_domain(c1, b1);
                        let d2 = realizable_draw_in_domain(c2, b2);
                        assert!(d1 <= s1, "market 1 drew past its own budget");
                        assert!(d2 <= s2, "market 2 drew past its own budget");
                        // DISJOINT: combined loss bounded by the SUM (no cross-subsidy).
                        assert!(d1 + d2 <= s1 + s2);
                        if s2 == 0 {
                            assert_eq!(d2, 0, "0-budget market must pay nothing");
                            if c2 > 0 {
                                saw_zero_budget = true;
                            }
                        }
                        if s1 > 0 && d1 == s1 {
                            saw_exhaust_one = true;
                        }
                        combos += 1;
                    }
                }
            }
        }
        assert!(combos > 100);
        assert!(saw_exhaust_one, "VACUOUS: never exhausted a single market's budget");
        assert!(saw_zero_budget, "VACUOUS: never tested a 0-budget non-subsidizing market");
    }
}
