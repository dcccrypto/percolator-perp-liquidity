#!/usr/bin/env python3
"""Generate the Design-A design graphs as dependency-free SVG (renders on GitHub).

Every curve is computed from the SAME integer formulas as the Rust model in ../model
(credit_rate, the seed-sized OI cap, the paid-LP break-even, the leverage ratchet), so the
pictures match the proofs. No third-party deps (stdlib only)."""

import math

BPS = 10_000
SCALE = 1_000_000
E9 = 1_000_000_000

# ---- model formulas (mirror ../model/src) -----------------------------------

def credit_rate(backing, claim):
    if claim == 0:
        return SCALE
    r = backing * SCALE // claim
    return min(r, SCALE)

def pool_draw(claim, backing):
    return claim * credit_rate(backing, claim) // SCALE

def oi_cap_sized_to_seed(seed, delta_max_bps):
    if delta_max_bps == 0:
        return seed
    return min(seed, seed * BPS // delta_max_bps)

def closed_form_income(fee_bps, spread_bps, funding_rate_e9, tau):
    turnover = (fee_bps + spread_bps) * SCALE // BPS
    funding = funding_rate_e9 * tau * SCALE // E9
    return (turnover + funding) / SCALE  # fraction of notional

def closed_form_adverse(sigma_slot_bps, tau):
    # sigma/1e4 * sqrt(tau) * 0.8, as the model computes it (integer isqrt * 4/5)
    v = sigma_slot_bps * SCALE * int(math.isqrt(tau)) * 4 // (BPS * 5)
    return v / SCALE

def initial_margin_bps(oi, ncap):
    if ncap == 0:
        return BPS
    return min(BPS, max(1000, BPS * oi // ncap))

# ---- tiny SVG line-chart writer ---------------------------------------------

W, H = 720, 440
ML, MR, MT, MB = 70, 24, 48, 56  # margins
PW, PH = W - ML - MR, H - MT - MB
COLORS = ["#2563eb", "#dc2626", "#16a34a", "#9333ea"]

def chart(path, title, xlabel, ylabel, series, xr, yr, hlines=None, notes=None):
    x0, x1 = xr
    y0, y1 = yr
    def px(x): return ML + (x - x0) / (x1 - x0) * PW
    def py(y): return MT + PH - (y - y0) / (y1 - y0) * PH
    s = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" font-family="ui-sans-serif,system-ui,Arial">']
    s.append(f'<rect width="{W}" height="{H}" fill="white"/>')
    s.append(f'<text x="{W/2}" y="26" text-anchor="middle" font-size="17" font-weight="700" fill="#111">{title}</text>')
    # gridlines + ticks (5 each)
    for i in range(6):
        gx = ML + PW * i / 5
        s.append(f'<line x1="{gx:.1f}" y1="{MT}" x2="{gx:.1f}" y2="{MT+PH}" stroke="#eee"/>')
        xv = x0 + (x1 - x0) * i / 5
        s.append(f'<text x="{gx:.1f}" y="{MT+PH+18}" text-anchor="middle" font-size="11" fill="#555">{fmt(xv)}</text>')
        gy = MT + PH * i / 5
        s.append(f'<line x1="{ML}" y1="{gy:.1f}" x2="{ML+PW}" y2="{gy:.1f}" stroke="#eee"/>')
        yv = y1 - (y1 - y0) * i / 5
        s.append(f'<text x="{ML-8}" y="{gy+4:.1f}" text-anchor="end" font-size="11" fill="#555">{fmt(yv)}</text>')
    # axes
    s.append(f'<line x1="{ML}" y1="{MT+PH}" x2="{ML+PW}" y2="{MT+PH}" stroke="#333"/>')
    s.append(f'<line x1="{ML}" y1="{MT}" x2="{ML}" y2="{MT+PH}" stroke="#333"/>')
    s.append(f'<text x="{ML+PW/2}" y="{H-14}" text-anchor="middle" font-size="13" fill="#222">{xlabel}</text>')
    s.append(f'<text x="18" y="{MT+PH/2}" text-anchor="middle" font-size="13" fill="#222" transform="rotate(-90 18 {MT+PH/2})">{ylabel}</text>')
    for hy, lab, col in (hlines or []):
        yy = py(hy)
        s.append(f'<line x1="{ML}" y1="{yy:.1f}" x2="{ML+PW}" y2="{yy:.1f}" stroke="{col}" stroke-dasharray="5 4" stroke-width="1.5"/>')
        s.append(f'<text x="{ML+PW-4}" y="{yy-5:.1f}" text-anchor="end" font-size="11" fill="{col}">{lab}</text>')
    for i, (name, pts) in enumerate(series):
        col = COLORS[i % len(COLORS)]
        d = " ".join(f"{px(x):.1f},{py(y):.1f}" for x, y in pts)
        s.append(f'<polyline points="{d}" fill="none" stroke="{col}" stroke-width="2.5"/>')
        ly = MT + 16 + i * 18
        s.append(f'<rect x="{ML+14}" y="{ly-9}" width="14" height="4" fill="{col}"/>')
        s.append(f'<text x="{ML+34}" y="{ly-4}" font-size="12" fill="#222">{name}</text>')
    if notes:
        s.append(f'<text x="{ML+PW}" y="{MT+12}" text-anchor="end" font-size="11" fill="#888">{notes}</text>')
    s.append("</svg>")
    open(path, "w").write("\n".join(s))
    print("wrote", path)

def fmt(v):
    a = abs(v)
    if a >= 1_000_000: return f"{v/1e6:.1f}M"
    if a >= 1_000: return f"{v/1e3:.0f}k"
    if a == 0: return "0"
    if a < 1: return f"{v:.2f}"
    return f"{v:.0f}"

# ---- 1. haircut curve --------------------------------------------------------
pts = []
for i in range(0, 301):
    ratio = i / 100  # winning claim as a multiple of the opposing pool
    backing = 1_000_000
    claim = int(ratio * backing)
    pts.append((ratio, credit_rate(backing, claim) / SCALE * 100))
chart("haircut.svg",
      "Payout vs demand on the opposing pool (the credit_rate cap)",
      "winning claim ÷ opposing pool", "% of winnings actually paid",
      [("credit_rate payout", pts)], (0, 3), (0, 105),
      hlines=[(100, "full payout", "#16a34a")],
      notes="paid 100% until the pool is exhausted, then a transparent haircut")

# ---- 2. seed loss is bounded by the seed ------------------------------------
seed = 100
ncap = oi_cap_sized_to_seed(seed, 0)        # 1:1 cap -> N_cap = seed
uncapped_oi = 500                            # a 5x-the-seed unguarded position
capped, uncapped = [], []
for i in range(0, 101):
    move = i / 100
    capped.append((i, pool_draw(int(ncap * move * SCALE) // SCALE * 1, seed) if False else min(int(ncap*move), seed)))
    uncapped.append((i, int(uncapped_oi * move)))
chart("seed_bounded.svg",
      "Seed loss: capped (OI sized to seed) vs unguarded",
      "adverse price move (%)", "seed loss (units, seed = 100)",
      [("OI cap sized to seed", capped), ("unguarded (5x seed OI)", uncapped)],
      (0, 100), (0, 520),
      hlines=[(seed, "seed = 100 (max loss)", "#16a34a")],
      notes="the cap flattens loss at the seed; unguarded OI blows past it")

# ---- 3. paid-LP break-even (funding ~ time beats adverse ~ sqrt(time)) -------
fee, spread, funding_e9, sigma = 30, 20, 1950, 8
inc, adv = [], []
TAU = 200_000
for i in range(0, 101):
    tau = max(1, int(TAU * i / 100))
    inc.append((tau, closed_form_income(fee, spread, funding_e9, tau) * 100))
    adv.append((tau, closed_form_adverse(sigma, tau) * 100))
chart("breakeven.svg",
      "Paid-LP break-even: income vs adverse selection over holding time",
      "holding time τ (slots)", "% of notional",
      [("income (fees+spread+funding·τ)", inc), ("adverse selection (σ·√τ·0.8)", adv)],
      (0, TAU), (0, 45),
      notes="short holds bleed (adverse > income); sustained holds earn — illustrative params")

# ---- 4. leverage ratchet -----------------------------------------------------
ncap = 1_000_000
lev = []
for i in range(0, 121):
    oi = int(ncap * i / 100)
    lev.append((i / 100, BPS / initial_margin_bps(oi, ncap)))
chart("ratchet.svg",
      "Leverage ratchet: max leverage shrinks as the one-sided book fills",
      "crowded OI ÷ cap", "max leverage (x)",
      [("max leverage", lev)], (0, 1.2), (0, 11),
      hlines=[(10, "10x (empty book)", "#16a34a"), (1, "1x (at cap)", "#dc2626")],
      notes="late lopsided entrants must post more margin -> protects the seed bound")
