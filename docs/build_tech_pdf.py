#!/usr/bin/env python3
"""Technical design deck (for feedback). Mechanism + math + diagrams + caveat resolutions.
No internal-process/verification-run references; file:line refs are design precision."""
import numpy as np, textwrap
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.backends.backend_pdf import PdfPages
from matplotlib.patches import FancyBboxPatch, FancyArrowPatch, Rectangle

INK="#1b2733"; SUB="#46555f"; TEAL="#2E6E78"; AMBER="#C57B36"; GRID="#DBD5CB"
GOOD="#2E8B57"; BAD="#B5482f"; NAVY="#27384A"; MONO="#33424f"
plt.rcParams.update({"font.family":"DejaVu Sans","font.size":11,"axes.edgecolor":SUB,
    "axes.labelcolor":INK,"text.color":INK,"xtick.color":SUB,"ytick.color":SUB,
    "figure.dpi":150,"savefig.dpi":150})
PAGE=(8.5,11.0)
OUT="/Users/khubair/percolator-perp-liquidity/docs/Perp-Liquidity-Technical-Design.pdf"

def page(title, kicker=""):
    fig=plt.figure(figsize=PAGE); fig.patch.set_facecolor("white")
    fig.add_artist(Rectangle((0,0.935),1,0.065,transform=fig.transFigure,color=TEAL,zorder=0))
    fig.add_artist(Rectangle((0,0.930),1,0.006,transform=fig.transFigure,color=AMBER,zorder=0))
    fig.text(0.06,0.962,title,color="white",fontsize=16,fontweight="bold",va="center")
    if kicker: fig.text(0.94,0.962,kicker,color="#cfe6ea",fontsize=9.5,va="center",ha="right")
    return fig

def foot(fig,n):
    fig.text(0.06,0.028,"Perp liquidity - technical design - draft for feedback",color=SUB,fontsize=8)
    fig.text(0.94,0.028,f"{n}",color=SUB,fontsize=9,ha="right")

def para(fig,x,y,text,width=92,size=10.6,color=INK,lh=0.0225,weight="normal"):
    lines=[]
    for blk in text.split("\n"):
        lines += (textwrap.wrap(blk,width=width) if blk.strip() else [""])
    for i,ln in enumerate(lines):
        fig.text(x,y-i*lh,ln,fontsize=size,color=color,fontweight=weight,va="top")
    return y-len(lines)*lh

def point(fig,x,y,head,body,width=86,size=10.4):
    fig.text(x,y,"•",fontsize=12,color=AMBER,va="top")
    fig.text(x+0.02,y,head,fontsize=size,color=INK,fontweight="bold",va="top")
    return para(fig,x+0.02,y-0.0215,body,width=width,size=size,color=SUB,lh=0.021)

def formula(fig,y,text,note="",size=13):
    fig.add_artist(Rectangle((0.10,y-0.034),0.80,0.052,transform=fig.transFigure,fc="#f3efe7",ec=GRID,lw=1))
    fig.text(0.5,y-0.008,text,ha="center",va="center",fontsize=size,color=NAVY,fontweight="bold")
    if note: fig.text(0.5,y-0.052,note,ha="center",fontsize=8.6,color=SUB,style="italic")
    return y-0.075

def code(fig,x,y,text,size=8.6,color=MONO):
    fig.text(x,y,text,fontsize=size,color=color,va="top",family="monospace")
    return y-0.019

def box(ax,cx,cy,w,h,text,fc,tc="white",fs=9.5,weight="bold"):
    ax.add_patch(FancyBboxPatch((cx-w/2,cy-h/2),w,h,boxstyle="round,pad=0.02,rounding_size=0.1",fc=fc,ec=fc,lw=1.2,zorder=2))
    ax.text(cx,cy,text,ha="center",va="center",color=tc,fontsize=fs,fontweight=weight,zorder=3)

def arrow(ax,x1,y1,x2,y2,c=SUB,lw=1.8):
    ax.add_patch(FancyArrowPatch((x1,y1),(x2,y2),arrowstyle="-|>",mutation_scale=13,color=c,lw=lw,zorder=1))

pages=[]

# ---- 1 TITLE ----
fig=plt.figure(figsize=PAGE); fig.patch.set_facecolor(NAVY)
fig.add_artist(Rectangle((0,0.64),1,0.01,transform=fig.transFigure,color=AMBER))
fig.text(0.5,0.73,"Permissionless Perps for Any Token",ha="center",color="white",fontsize=24,fontweight="bold")
fig.text(0.5,0.585,"Technical design: mechanism, math, and how the\nopen safety problems get resolved",ha="center",color="#cfe6ea",fontsize=14,linespacing=1.6)
fig.text(0.5,0.30,"Grounded in the live engine. Draft for technical feedback.",ha="center",color="#9fc7cd",fontsize=11.5,style="italic")
fig.text(0.5,0.085,"2026",ha="center",color="#7f95a3",fontsize=10)
pages.append(fig)

# ---- 2 OVERVIEW + CLAIM ----
fig=page("Overview and the claim","what we're asserting")
y=0.875
y=para(fig,0.06,y,"The goal: let any project open a tradable perp on its token with a small, refundable creator stake "
 "instead of deep liquidity or paid market makers. An automated curve is the counterparty from block one; "
 "a creator first-loss buffer is the real money behind the trades; the market grows from its own fees.")
y-=0.012
fig.text(0.06,y,"The safety claim, precisely:",fontsize=12,color=TEAL,fontweight="bold",va="top"); y-=0.03
y=para(fig,0.06,y,"For an isolated market, the protocol and non-creator capital take no loss, and the most that can be "
 "extracted is bounded by the market's own capital C_m, with the creator's stake consumed first. Capacity "
 "is bounded by:",size=10.6)
y-=0.006
y=formula(fig,y-0.01,"N_max  =  β · C_m / R_max",
          "C_m = creator stake + locked fees + extra backing.  β<1 safety margin.  R_max = max loss per position.")
fig.text(0.06,y,"What's true today vs what needs building:",fontsize=12,color=TEAL,fontweight="bold",va="top"); y-=0.028
y=point(fig,0.065,y,"The fee / pricing engine is sound now.","The vol-adaptive curve and matched-book settlement work on the live engine.")
y-=0.004
y=point(fig,0.065,y,"The capacity bound is not yet enforced in the core engine.","The four sections that follow turn each open assumption into a concrete, code-level fix with a known residual.")
foot(fig,2); pages.append(fig)

# ---- 3 ENGINE FOUNDATION ----
fig=page("The engine it's built on","matched book + a real counterparty")
ax=fig.add_axes([0.05,0.40,0.9,0.46]); ax.set_xlim(0,10); ax.set_ylim(0,10); ax.axis("off")
box(ax,1.7,8.5,2.6,1.2,"Taker order",AMBER)
box(ax,5.0,8.5,2.6,1.2,"Matcher\n(optional maker auction)",TEAL,fs=8.5)
box(ax,8.3,8.5,2.6,1.2,"Core engine\nmatched book",NAVY,fs=9)
arrow(ax,3.0,8.5,3.7,8.5); arrow(ax,6.3,8.5,7.0,8.5)
box(ax,5.0,5.6,8.4,1.3,"Invariant: oi_eff_long_q == oi_eff_short_q  (net skew = 0)   [v16.rs:4910]","#5b6b54",fs=9)
arrow(ax,8.3,7.9,5.4,6.25)
box(ax,2.3,3.0,3.4,1.3,"LP / creator-backed\nvault = the other side",BAD,fs=9)
box(ax,7.2,3.0,3.6,1.3,"EWMA mark from the\nmarket's own trades",AMBER,fs=9)
arrow(ax,5.0,4.95,2.3,3.65,c=GRID); arrow(ax,5.0,4.95,7.2,3.65,c=GRID)
y=0.36
y=para(fig,0.06,y,"The book is always net-flat: every long is matched by a short. The creator-backed vault account IS the "
 "counterparty that fills the gap, so no external market maker is needed (v16_program.rs:7299). For a token "
 "with no oracle, the mark is an EWMA of the market's own trades (ORACLE_MODE_EWMA_MARK). Anyone can crank "
 "the market permissionlessly to keep it current. This is the foundation the design rests on; it is real and "
 "live. What it lacks - a hard capacity cap, a locked buffer, segregated insurance, an ordered loss "
 "waterfall - is what the next sections add.")
foot(fig,3); pages.append(fig)

# ---- 4 MECHANISM ----
fig=page("The mechanism","curve, fee, capacity")
y=0.875
y=para(fig,0.06,y,"A trade moves the quoted price along a constant-product curve; the fee adapts to volatility; and the "
 "size that can be opened is bounded by the buffer. The three pieces:")
y-=0.008
y=point(fig,0.065,y,"Pricing curve.","Integer constant-product quoting around the EWMA mark, with a per-slot price clamp so no single slot can move the mark more than a set fraction.")
y-=0.003
y=point(fig,0.065,y,"Adaptive fee.","Fee rises with measured volatility (next page). Cheap when calm to attract flow, expensive when wild to protect the buffer and to make manipulation uneconomic.")
y-=0.003
y=point(fig,0.065,y,"Capacity cap.","Total open interest is capped so the worst-case loss the market can owe stays inside the buffer. This is the load-bearing safety bound:")
y-=0.006
y=formula(fig,y-0.008,"open interest  ≤  N_max  =  β · C_m / R_max",
          "Enforced as a hard reject at trade preflight (proposed; see Fix 1). Today this is unenforced in the core engine.")
y=para(fig,0.06,y,"Everything downstream - the loss waterfall, the liquidation budget, the manipulation cost - is sized "
 "against this same N_max. If the cap holds, the rest of the safety argument is a matter of ordering and "
 "timing, which the fixes address.",size=10.4,color=SUB)
foot(fig,4); pages.append(fig)

# ---- 5 FEE MATH ----
fig=page("The adaptive fee","the part that's validated today")
y=0.875
y=para(fig,0.06,y,"The fee tracks a running estimate of the token's volatility. In a simple, integer-friendly form:")
y=formula(fig,y-0.01,"fee_bps  =  clamp( a + b · σ̂ + c · σ̂² ,  floor ,  ceiling )",
          "σ̂ = running volatility estimate (cumulative-mean variance).  Calm → near the floor; wild → toward the ceiling.")
ax=fig.add_axes([0.16,0.30,0.7,0.28])
v=np.linspace(0,1,100); feec=20+55*v+30*v*v
ax.plot(v,feec,color=TEAL,lw=2.6,label="adaptive")
ax.axhline(65,color=SUB,ls="--",lw=1.3,label="best fixed fee")
ax.set_xlabel("volatility  σ̂  ->"); ax.set_ylabel("fee (bps)"); ax.set_xticks([]); ax.set_ylim(0,120)
ax.grid(True,color=GRID,lw=0.5); ax.legend(fontsize=9,loc="upper left",frameon=False)
para(fig,0.06,0.235,"In simulation across many markets, this adaptive rule beat the best single fixed fee by roughly 6% on "
 "captured spread while staying competitive, with bit-for-bit identical behaviour in native and on-chain "
 "execution. It charges more only where the risk actually is, which is also what makes the mark expensive to "
 "manipulate (Fix 3).",width=92,size=10.2,color=SUB,lh=0.0215)
foot(fig,5); pages.append(fig)

# ---- 6 WATERFALL ----
fig=page("Capital stack and loss waterfall","who absorbs a loss, in what order")
ax=fig.add_axes([0.05,0.36,0.9,0.50]); ax.set_xlim(0,10); ax.set_ylim(0,10); ax.axis("off")
rows=[("Losing trader's margin","first, always","#3a4a5a"),
      ("Creator first-loss stake  S","skin in the game; consumed before any LP","#7a3b2e"),
      ("Market insurance  I_m  (segregated)","this market's own fund; can't touch other markets","#b06a36"),
      ("ADL / proportional haircut","tail only; visible on-chain; never a freeze","#6a4a8a")]
ytop=9.2; h=1.7
for i,(t,d,c) in enumerate(rows):
    yy=ytop-i*(h+0.18)
    ax.add_patch(FancyBboxPatch((1.3,yy-h),6.6,h,boxstyle="round,pad=0.02,rounding_size=0.09",fc=c,ec=c))
    ax.text(1.6,yy-h/2+0.24,t,color="white",fontsize=10.5,fontweight="bold",va="center")
    ax.text(1.6,yy-h/2-0.30,d,color="#f3ece2",fontsize=8.8,va="center")
arrow(ax,8.4,9.0,8.4,1.6,c=BAD,lw=2.2); ax.text(8.7,5.3,"consumed top-down",color=BAD,rotation=90,va="center",fontsize=9,fontweight="bold")
y=0.32
para(fig,0.06,y,"The promise to traders: in normal conditions winners are paid in full by the losing side. Only when claims "
 "exceed the whole stack does the bottom layer engage, as a proportional, on-chain-readable haircut - not a "
 "freeze, never zero. The key correctness requirement is ORDER: the creator's stake must be spent before any "
 "non-creator LP is touched. On the live engine today, the bottom layer (ADL) can fire before the creator "
 "layer is exhausted. Fix 1b reorders this.",width=92)
foot(fig,6); pages.append(fig)

# ---- 7 FIX 1a ----
fig=page("Fix 1a - hard OI cap + locked backing","close the capacity & self-funding holes")
y=0.878
fig.text(0.06,y,"Problem.",fontsize=11,color=BAD,fontweight="bold",va="top")
y=para(fig,0.155,y,"Gross OI is unbounded in the core engine (the net-flat invariant bounds skew, not size). And backing can "
 "be withdrawn while positions are open, because the withdrawal gate keys on converted PnL, not open OI "
 "(v16_program.rs:8350) - so a bond posted to inflate the cap can be pulled right back, dumping the loss on "
 "everyone else.",width=86,size=10.2)
y-=0.008
fig.text(0.06,y,"Fix.",fontsize=11,color=GOOD,fontweight="bold",va="top"); y-=0.026
y=code(fig,0.07,y,"// at trade preflight (v16.rs ~10009)")
y=code(fig,0.07,y,"if cap != 0 && oi_eff_long_q + size_q > gross_oi_cap_q { reject }")
y=code(fig,0.07,y,"// at backing withdrawal (v16_program.rs:8350)")
y=code(fig,0.07,y,"require reserved_after ≥ reserved · side_OI / cap")
y-=0.006
y=formula(fig,y-0.006,"withdrawable  =  reserved · ( 1 − side_OI / N_max )",
          "You can only ever pull the un-encumbered fraction. At full OI, nothing; as positions close, it frees up.")
fig.text(0.06,y,"Adversarial re-check & residual.",fontsize=11,color=TEAL,fontweight="bold",va="top"); y-=0.026
y=para(fig,0.06,y,"Re-running the self-funding attack: the bond can no longer be withdrawn while it backs open OI - rejected "
 "at the floor. Two residuals, both handled: the cap must be tied to current backing (not settable to "
 "infinity), and a same-block withdraw-then-open race is itself bounded by the OI cap. Cost: ~200 lines, one "
 "new field per market (layout migration), a handful of machine-checkable invariants.",width=92,size=10.2)
foot(fig,7); pages.append(fig)

# ---- 8 FIX 1b ----
fig=page("Fix 1b - ordered waterfall + segregation","creator pays first; markets isolated")
y=0.878
fig.text(0.06,y,"Problem.",fontsize=11,color=BAD,fontweight="bold",va="top")
y=para(fig,0.155,y,"On bankruptcy, the social-loss step adjusts the opposite side's index (v16.rs:9084/9525), writing losses "
 "onto non-creator LPs before the creator stake is used up. And insurance is one shared pool per group "
 "(v16_program.rs:4606), so one market can drain another.",width=86,size=10.2)
y-=0.008
fig.text(0.06,y,"Fix.",fontsize=11,color=GOOD,fontweight="bold",va="top"); y-=0.024
y=para(fig,0.06,y,"Insert a creator-funded layer that is consumed BETWEEN insurance and ADL (v16.rs ~9737), so ADL only "
 "fires once the creator stake hits zero. Replace the shared pool with per-market segregated insurance held "
 "in a per-market vault PDA, so a loss in market A cannot reduce market B's fund.",width=92,size=10.2)
y-=0.006
# before/after worked example
ax=fig.add_axes([0.06,0.30,0.88,0.20]); ax.set_xlim(0,10); ax.set_ylim(0,10); ax.axis("off")
ax.text(2.4,9,"BEFORE",ha="center",fontsize=10,color=BAD,fontweight="bold")
ax.text(7.6,9,"AFTER (creator-first)",ha="center",fontsize=10,color=GOOD,fontweight="bold")
ax.add_patch(FancyBboxPatch((0.4,1),4.0,6.6,boxstyle="round,pad=0.05,rounding_size=0.2",fc="#fbf1ee",ec=BAD,lw=1.5))
ax.add_patch(FancyBboxPatch((5.6,1),4.0,6.6,boxstyle="round,pad=0.05,rounding_size=0.2",fc="#eef6f0",ec=GOOD,lw=1.5))
for i,(t,c) in enumerate([("S=$50, I_m=$100, LP=$500",INK),("3 bankrupt longs ($300)",INK),
                          ("ADL hits LP early",BAD),("LP loses  $200",BAD)]):
    ax.text(0.7,7.0-i*1.5,t,fontsize=8.8,color=c,va="center",fontweight=("bold" if i==3 else "normal"))
for i,(t,c) in enumerate([("S=$50, I_m=$100, LP=$500",INK),("insurance, then S, then ADL",INK),
                          ("creator $50 absorbed first",GOOD),("LP loses  $60",GOOD)]):
    ax.text(5.9,7.0-i*1.5,t,fontsize=8.8,color=c,va="center",fontweight=("bold" if i==3 else "normal"))
y=0.265
para(fig,0.06,y,"Residual: in the deepest tail (creator stake AND insurance both exhausted) ADL still reaches LPs - by "
 "design, and only there. The creator stake must be OI-locked (same lock as Fix 1a) so it can't be pulled "
 "before it's needed. Cost: ~4 new fields/market, a new vault PDA, a one-time migration.",width=92,size=10.2)
foot(fig,8); pages.append(fig)

# ---- 9 FIX 2 LIQUIDATION ----
fig=page("Fix 2 - liquidation under stress","the sobering one, made bounded")
y=0.878
y=para(fig,0.06,y,"Close-out is one account, one leg, per transaction; a price-moving crank invalidates other accounts' "
 "health certs; a too-fast move halts the market (RecoveryRequired). If close-out lags, the per-position loss "
 "grows. The honest bound, with a correctly-batched keeper:",size=10.4)
y=formula(fig,y-0.008,"C_m  ≥  OI_cap · P · 2L / k",
          "P = per-slot price clamp,  L = max legs per account,  k = fraction of slots the keeper lands a tx.")
ax=fig.add_axes([0.13,0.32,0.5,0.24])
L=np.arange(1,15); ratio=0.05*2*L/0.5
ax.plot(L,ratio,color=BAD,lw=2.4,marker="o",ms=3)
ax.set_xlabel("legs per account  L"); ax.set_ylabel("required  C_m / OI_cap"); ax.grid(True,color=GRID,lw=0.5)
ax.axhline(1,color=SUB,ls="--",lw=1); ax.set_title("capital ratio grows with leg count",fontsize=9,color=SUB)
fig.text(0.67,0.52,"At L=14 the bound\nis ~290x worse\nunder congestion.",fontsize=9,color=BAD,va="top")
fig.text(0.67,0.45,"Cap L≤4, tighten\nthe clamp, keep\nOI small vs C_m\n→ ratio ~2-7x.",fontsize=9,color=GOOD,va="top")
y=0.27
para(fig,0.06,y,"So this caveat is closed mostly by CONFIG, not code: cap legs at 4, set a conservative per-slot clamp, "
 "size OI small relative to the buffer, and run the keeper one-leg-per-account in parallel. Under extreme "
 "congestion or censorship the market HALTS safely (no new loss accrues) rather than staying open. The price "
 "of this safety is capital efficiency: you hold a few times more buffer than open interest.",width=92,size=10.2)
foot(fig,9); pages.append(fig)

# ---- 10 FIX 3 MARK ----
fig=page("Fix 3 - mark manipulation","internal: solved.  external: discipline.")
y=0.878
fig.text(0.06,y,"Internal (own-market).",fontsize=11,color=TEAL,fontweight="bold",va="top"); y-=0.024
y=para(fig,0.06,y,"A depth gate keeps size capped until the market clears a minimum age and volume; a longer EWMA half-life "
 "and tight per-slot clamp slow the mark; the OI cap bounds the prize. Together the cost to move the mark "
 "exceeds the capped gain:",size=10.2)
y=formula(fig,y-0.006,"cost to move mark   >   max extractable  ≤  C_m",
          "N_max must be PROTOCOL-enforced, not creator-set - else a creator-as-manipulator breaks even.")
fig.text(0.06,y,"External (other protocols).",fontsize=11,color=AMBER,fontweight="bold",va="top"); y-=0.024
y=para(fig,0.06,y,"Zero out the public oracle_target_price_e6 in endogenous-mark mode (sentinel), set a discriminator flag, "
 "and warn in the IDL. This stops NAIVE misuse cold. It does NOT stop a determined third party from reading "
 "the public mark directly - nothing on-chain can. We classify external use as a documented "
 "integrator-discipline risk and state it plainly.",size=10.2)
y-=0.008
fig.text(0.06,y,"Residual.",fontsize=11,color=BAD,fontweight="bold",va="top"); y-=0.024
y=para(fig,0.06,y,"A time gate (e.g. several hours) is the hardest control - a wash-trading ring can pay fees to inflate "
 "volume but cannot skip the clock. Cost: ~110 lines, a few per-market config knobs, no change to the "
 "matched-book core.",size=10.2)
foot(fig,10); pages.append(fig)

# ---- 11 FIX 4 PREFUNDED ----
fig=page("Fix 4 - the always-paid version","when capped haircuts aren't acceptable")
y=0.878
y=para(fig,0.06,y,"For markets that want to promise every winner a full payout no matter what, the haircut tail has to be "
 "removed - which means the maximum payout is escrowed up front. The payoff per position is capped, and the "
 "reserve covers the whole cap:")
y=formula(fig,y-0.01,"Reserve  =  OI_cap · (payout cap multiple)     escrowed before trading",
          "With the max owed pre-funded, the ADL / haircut path is never reached. This is the only mode that is fully safe today.")
ax=fig.add_axes([0.1,0.34,0.8,0.20]); ax.set_xlim(0,10); ax.set_ylim(0,10); ax.axis("off")
box(ax,2.0,5,3.2,3.2,"Escrow the\nmax payout\nup front",GOOD,fs=9.5)
arrow(ax,3.7,5,5.0,5)
box(ax,6.6,5,5.0,3.2,"Winners always paid in full,\nprotocol can't go negative,\ncap limits a single win",NAVY,fs=8.8)
y=0.30
para(fig,0.06,y,"The trade-off is explicit: it costs real capital and caps the upside on any one position, so it suits "
 "tokens or venues that already have funds to reserve and want a bullet-proof guarantee. The cheap, "
 "capped-haircut version (Fixes 1-3) is for the long-tail case where pre-funding isn't available; the "
 "pre-funded version is the opt-in upgrade for those who can afford certainty.",width=92)
foot(fig,11); pages.append(fig)

# ---- 12 RESIDUALS ----
fig=page("Residual risks","stated plainly")
y=0.875
for h,b in [
 ("Capital efficiency.","Safety against lagging liquidation means holding a few times more buffer than open interest. Long-tail markets stay small until they prove out."),
 ("Deepest tail still reaches LPs.","Once creator stake and segregated insurance are both exhausted, ADL touches LPs - by design. Only the pre-funded mode removes this entirely."),
 ("Authority / key management.","The OI cap and creator-stake controls are only as safe as the keys that set them. Standard multisig / governance discipline applies."),
 ("Determined external mark misuse.","Hygiene stops accidents, not a determined integrator reading the public mark. This cannot be fixed on-chain; it is documented, not eliminated."),
 ("Congestion / censorship.","Under sustained censorship the market halts safely rather than staying open. Liveness, not solvency, is what degrades."),
 ("Migration.","Existing markets need a one-time, frozen-state migration to the segregated-insurance layout. New markets get it from launch."),
]:
    y=point(fig,0.06,y,h,b,width=90,size=10.3); y-=0.008
foot(fig,12); pages.append(fig)

# ---- 13 BUILD PATH ----
fig=page("Build path","config now, then code, then migrate")
ax=fig.add_axes([0.05,0.40,0.9,0.46]); ax.set_xlim(0,10); ax.set_ylim(0,10); ax.axis("off")
steps=[("Now (config only)","cap legs ≤4, tight per-slot clamp, conservative OI/C_m ratio, keeper batching, mark hygiene defaults","#2E6E78"),
       ("Code round 1","hard OI cap + locked backing + ordered creator-first waterfall (Fixes 1a/1b)","#9a7d2e"),
       ("Code round 2","per-market segregated insurance + dual-side vault + depth-gated mark (Fix 3)","#7a3b2e"),
       ("Migrate + optional","one-time layout migration; offer the pre-funded capped mode (Fix 4) to those who want it","#6a4a8a")]
y0=9.0
for i,(t,d,c) in enumerate(steps):
    yy=y0-i*2.3
    ax.add_patch(FancyBboxPatch((0.3,yy-1.9),1.1,1.9,boxstyle="round,pad=0.03,rounding_size=0.1",fc=c,ec=c))
    ax.text(0.85,yy-0.95,str(i),color="white",fontsize=16,fontweight="bold",ha="center",va="center")
    ax.add_patch(FancyBboxPatch((1.7,yy-1.9),7.9,1.9,boxstyle="round,pad=0.03,rounding_size=0.05",fc="white",ec=c,lw=1.4))
    ax.text(1.95,yy-0.5,t,color=c,fontsize=10.5,fontweight="bold",va="top")
    for j,ln in enumerate(textwrap.wrap(d,82)): ax.text(1.95,yy-0.98-j*0.33,ln,color=SUB,fontsize=9,va="top")
    if i<3: arrow(ax,0.85,yy-1.95,0.85,yy-2.3,c=c,lw=1.7)
para(fig,0.06,0.34,"Rough size of the coded work: a few hundred lines across the engine and program, four to six new "
 "per-market fields, two new instructions, one migration, and a set of machine-checkable invariants for the "
 "cap, the waterfall order, and the segregation. None of it touches the matched-book core's settlement math.",width=92,size=10.3)
foot(fig,13); pages.append(fig)

# ---- 14 QUESTIONS ----
fig=page("Open questions for feedback","where your read helps most")
y=0.86
for h,b in [
 ("Capital ratio.","Is a 2-7x buffer-to-OI ratio acceptable for a long-tail launch, or does that kill the use case versus a smaller, capped product?"),
 ("Capped vs uncapped payout.","For a brand-new token, will traders accept a rare, visible, proportional haircut in exchange for cheap launch, or do they expect the pre-funded guarantee?"),
 ("Liquidation under congestion.","Is halt-then-recover an acceptable failure mode, and is the keeper-batching assumption realistic on mainnet?"),
 ("Mark trust.","Is the depth-gate + time-gate enough for the market's own safety, given external misuse stays a documented risk?"),
 ("Scope.","Which of the four fixes is worth building first, and is anything missing from the safety argument?"),
]:
    y=point(fig,0.06,y,h,b,width=90,size=10.5); y-=0.012
foot(fig,14); pages.append(fig)

with PdfPages(OUT) as pdf:
    import os; os.makedirs("/tmp/tech_png",exist_ok=True)
    for i,f in enumerate(pages,1):
        pdf.savefig(f,facecolor=f.get_facecolor()); f.savefig(f"/tmp/tech_png/t{i:02d}.png",facecolor=f.get_facecolor(),dpi=110); plt.close(f)
print("WROTE",OUT,"pages:",len(pages))
