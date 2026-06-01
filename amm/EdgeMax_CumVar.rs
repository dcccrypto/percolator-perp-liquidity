use pinocchio::{account_info::AccountInfo, entrypoint, pubkey::Pubkey, ProgramResult};
use prop_amm_submission_sdk::{set_return_data_bytes, set_return_data_u64, set_storage};

const NAME: &str = "EdgeMax CumVar";
const MODEL_USED: &str = "Claude Opus 4.8";

// ============================================================================
// EdgeMax (CumVar) — vol-adaptive fee with a CUMULATIVE-MEAN variance estimator.
//
// The whole edge is: the competitor normalizer is a FIXED-fee CPMM, but the
// per-sim GBM volatility sigma (fixed for all 10k steps, drawn in [1bp,70bp])
// is unknown and varies sim to sim. We estimate sigma online and set our fee to
// the level that best trades retail-spread capture (POSITIVE edge) against LVR
// paid to the arbitrageur (NEGATIVE edge): cheap at low vol (capture the spread
// the fixed-fee normalizer over-charges), steep at high vol (defend against LVR).
//
// ESTIMATOR — cumulative mean, not EWMA. Because sigma is STATIONARY for the
// entire sim, the maximum-likelihood estimate of the variance is the simple
// average of the per-step squared relative moves; its estimation error -> 0 as
// samples accumulate. A fixed-alpha EWMA has an irreducible noise floor and is
// strictly worse here.
//
//   variance  = var_sum / count            (bps^2, integer)
//   sigma_hat = isqrt(variance)            (bps)
//   fee_bps   = LO + A_NUM*sigma_hat/A_DEN + sigma_hat^2/B_DEN   (clamped LO..HI)
//
// SAMPLING — the arbitrageur runs against us FIRST every step and corrects our
// reserve-implied price toward fair, so on the first executed trade of a NEW
// step p = reserve_y/reserve_x ~= fair. We sample the relative move since the
// previous step boundary exactly there (one clean sample per step).
//
// SHAPE SAFETY — fee_from_state reads ONLY reserves + storage, never the input
// amount, so each side's quote is a pure fee-discounted constant-product curve,
// which is provably monotone + concave in input.
//
// PARITY — the BPF tag-2 path decodes storage from data[42..], runs the SAME
// after_swap on a local buffer, and persists via set_storage, so the native and
// BPF fee trajectories are identical (validator native/BPF parity delta = 0).
// ============================================================================

// ---- estimator / fixed-point tunables --------------------------------------
const P_SCALE: u128 = 1_000_000_000; // price fixed-point: p_fp = ry*P_SCALE/rx
const R2_CAP: u128 = 62_500; // cap one squared per-step move at (250 bps)^2
const MOVE_BPS_CAP: u128 = 250; // cap the per-step relative move BEFORE squaring

// ---- vol -> fee mapping (bps) ----------------------------------------------
const FEE_LO: u128 = 20; // fee floor (LVR floor; lower leaks to arb)
const FEE_HI: u128 = 130; // fee ceiling (rarely binding)
const A_NUM: u128 = 7; // linear term  = 0.7 * sigma_hat
const A_DEN: u128 = 10;
const B_DEN: u128 = 160; // quadratic term = sigma_hat^2 / 200
const COLD_FEE: u128 = 55; // default fee until the estimator warms up
const WARMUP_STEPS: u64 = 16; // per-step samples required before trusting sigma_hat

// Hard guard for cp_out so the (10_000 - fee_bps) term can never underflow even
// if a future edit feeds it an out-of-range fee. fee_from_state already clamps
// to [FEE_LO, FEE_HI], so this is belt-and-suspenders and behaviour-preserving.
const FEE_HARD_MAX: u128 = 9_900;

// ---- storage byte layout (1024 bytes total, little-endian) ------------------
//   [0..8]   MAGIC          u64   — "initialized" sentinel
//   [8..16]  last_step      u64   — step index of last sampled price
//   [16..32] last_price_fp  u128  — reserve_y*P_SCALE/reserve_x at last sample
//   [32..48] var_sum        u128  — running sum of capped (dp/p)^2 in bps^2
//   [48..56] sample_count   u64   — number of per-step samples taken
//   [56..1024] unused (zeroed)
const OFF_MAGIC: usize = 0;
const OFF_LAST_STEP: usize = 8;
const OFF_LAST_PRICE: usize = 16;
const OFF_VAR_SUM: usize = 32;
const OFF_SAMPLE_COUNT: usize = 48;
const STATE_END: usize = 56; // last byte offset we touch (exclusive)
const MAGIC: u64 = 0x4544_4745_4D41_5832; // "EDGEMAX2"-ish sentinel

#[cfg(not(feature = "no-entrypoint"))]
entrypoint!(process_instruction);

pub fn process_instruction(_pid: &Pubkey, _a: &[AccountInfo], data: &[u8]) -> ProgramResult {
    if data.is_empty() {
        return Ok(());
    }
    match data[0] {
        0 | 1 => set_return_data_u64(compute_swap(data)),
        2 => {
            // BPF mirror of native after_swap: decode current storage from the
            // instruction, run the SAME logic on a local buffer, then persist via
            // set_storage so the fee adapts identically on native and BPF.
            if data.len() >= 42 + 1024 {
                let mut s = [0u8; 1024];
                s.copy_from_slice(&data[42..42 + 1024]);
                after_swap(data, &mut s);
                let _ = set_storage(&s);
            }
        }
        3 => set_return_data_bytes(NAME.as_bytes()),
        4 => set_return_data_bytes(get_model_used().as_bytes()),
        _ => {}
    }
    Ok(())
}

pub fn get_model_used() -> &'static str {
    MODEL_USED
}

// ---- little-endian helpers --------------------------------------------------
#[inline]
fn rd_u64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes([
        b[o], b[o + 1], b[o + 2], b[o + 3], b[o + 4], b[o + 5], b[o + 6], b[o + 7],
    ])
}

#[inline]
fn rd8(b: &[u8], o: usize) -> u128 {
    rd_u64(b, o) as u128
}

#[inline]
fn rd16(b: &[u8], o: usize) -> u128 {
    u128::from_le_bytes([
        b[o], b[o + 1], b[o + 2], b[o + 3], b[o + 4], b[o + 5], b[o + 6], b[o + 7],
        b[o + 8], b[o + 9], b[o + 10], b[o + 11], b[o + 12], b[o + 13], b[o + 14], b[o + 15],
    ])
}

#[inline]
fn wr_u64(b: &mut [u8], o: usize, v: u64) {
    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

#[inline]
fn wr16(b: &mut [u8], o: usize, v: u128) {
    b[o..o + 16].copy_from_slice(&v.to_le_bytes());
}

// ---- integer sqrt (Newton, u128) -------------------------------------------
#[inline]
fn isqrt(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

// ---- CPMM-with-fee output (monotone + concave; fee independent of input) ----
#[inline]
fn cp_out(side: u8, input: u128, rx: u128, ry: u128, fee_bps: u128) -> u64 {
    if rx == 0 || ry == 0 || input == 0 {
        return 0;
    }
    // Defensive clamp: callers pass a fee already in [FEE_LO, FEE_HI], but never
    // let 10_000 - fee_bps underflow regardless of how cp_out gets called.
    let fee = if fee_bps > FEE_HARD_MAX {
        FEE_HARD_MAX
    } else {
        fee_bps
    };
    // u128 math: input <= ~1.8e19, (10_000 - fee) <= 10_000 -> product <= ~1.8e23,
    // well within u128. k = rx*ry can reach ~1e26 (still < u128::MAX ~3.4e38).
    let net = input * (10_000 - fee) / 10_000;
    if net == 0 {
        return 0;
    }
    let k = rx * ry;
    match side {
        0 => {
            let n = ry + net;
            rx.saturating_sub((k + n - 1) / n) as u64
        }
        1 => {
            let n = rx + net;
            ry.saturating_sub((k + n - 1) / n) as u64
        }
        _ => 0,
    }
}

pub fn compute_swap(data: &[u8]) -> u64 {
    if data.len() < 25 {
        return 0;
    }
    let side = data[0];
    if side != 0 && side != 1 {
        return 0;
    }
    let input = rd8(data, 1);
    let rx = rd8(data, 9);
    let ry = rd8(data, 17);
    let storage: &[u8] = if data.len() >= 25 + 1024 {
        &data[25..25 + 1024]
    } else {
        &[]
    };
    let fee_bps = fee_from_state(side, rx, ry, storage); // MUST NOT depend on input
    cp_out(side, input, rx, ry, fee_bps)
}

/// Fee in bps. Depends ONLY on reserves + storage (never on input_amount), so
/// the resulting CPMM curve is monotone + concave per side.
fn fee_from_state(_side: u8, _rx: u128, _ry: u128, storage: &[u8]) -> u128 {
    if storage.len() < STATE_END {
        return COLD_FEE;
    }
    let magic = rd_u64(storage, OFF_MAGIC);
    let count = rd_u64(storage, OFF_SAMPLE_COUNT);
    if magic != MAGIC || count < WARMUP_STEPS {
        // Cold start / warmup: not enough samples to trust the estimate yet.
        return COLD_FEE;
    }
    let var_sum = rd16(storage, OFF_VAR_SUM);
    let variance = var_sum / (count as u128); // MLE of stationary variance (bps^2)
    let sigma_hat = isqrt(variance); // bps

    // fee = LO + A*sigma_hat + sigma_hat^2/B_DEN
    let lin = A_NUM * sigma_hat / A_DEN;
    let quad = sigma_hat * sigma_hat / B_DEN;
    let mut fee = FEE_LO + lin + quad;
    if fee < FEE_LO {
        fee = FEE_LO;
    }
    if fee > FEE_HI {
        fee = FEE_HI;
    }
    fee
}

/// Called after EVERY executed trade. We SAMPLE the reserve-implied price once
/// per step — on the first trade whose `step` field has advanced past the last
/// sampled step. That first trade is typically the arbitrageur's correction, so
/// the reserve ratio ~= fair at that instant, giving a clean per-step move.
pub fn after_swap(data: &[u8], storage: &mut [u8]) {
    if data.len() < 42 || storage.len() < STATE_END {
        return;
    }
    // POST-trade reserves and the step index for this trade.
    let rx = rd8(data, 18);
    let ry = rd8(data, 26);
    let step = rd_u64(data, 34);
    if rx == 0 || ry == 0 {
        return;
    }

    let magic = rd_u64(storage, OFF_MAGIC);
    let price_fp = ry.saturating_mul(P_SCALE) / rx;

    if magic != MAGIC {
        // First ever call: initialize. No move sample yet (this is the step-0
        // arb correction, used only to anchor the price).
        wr_u64(storage, OFF_MAGIC, MAGIC);
        wr_u64(storage, OFF_LAST_STEP, step);
        wr16(storage, OFF_LAST_PRICE, price_fp);
        wr16(storage, OFF_VAR_SUM, 0);
        wr_u64(storage, OFF_SAMPLE_COUNT, 0);
        return;
    }

    let last_step = rd_u64(storage, OFF_LAST_STEP);
    // Only the FIRST trade of a NEW step produces a per-step sample.
    if step <= last_step {
        return;
    }

    let last_price = rd16(storage, OFF_LAST_PRICE);
    if last_price == 0 {
        // Defensive: re-anchor without a sample.
        wr_u64(storage, OFF_LAST_STEP, step);
        wr16(storage, OFF_LAST_PRICE, price_fp);
        return;
    }

    // Relative move in bps: |price_fp - last_price| * 10000 / last_price
    let diff = if price_fp >= last_price {
        price_fp - last_price
    } else {
        last_price - price_fp
    };
    // Clamp the move BEFORE squaring so the square can never approach u128::MAX
    // even in the validator's most extreme synthetic reserve/last_price regimes.
    let mut move_bps = diff.saturating_mul(10_000) / last_price;
    if move_bps > MOVE_BPS_CAP {
        move_bps = MOVE_BPS_CAP;
    }
    let mut r2 = move_bps.saturating_mul(move_bps); // bps^2, <= R2_CAP by construction
    if r2 > R2_CAP {
        r2 = R2_CAP; // redundant given the move clamp, kept as a hard ceiling
    }

    // Cumulative-mean variance: accumulate sum + count.
    let var_sum = rd16(storage, OFF_VAR_SUM).saturating_add(r2);
    let count = rd_u64(storage, OFF_SAMPLE_COUNT).saturating_add(1);

    wr_u64(storage, OFF_LAST_STEP, step);
    wr16(storage, OFF_LAST_PRICE, price_fp);
    wr16(storage, OFF_VAR_SUM, var_sum);
    wr_u64(storage, OFF_SAMPLE_COUNT, count);
}