#!/usr/bin/env python3
"""Clean, shareable design doc (for feedback). Human-voiced; no internal-process/verification refs."""
import numpy as np, textwrap
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.backends.backend_pdf import PdfPages
from matplotlib.patches import FancyBboxPatch, FancyArrowPatch, Rectangle

INK="#1b2733"; SUB="#46555f"; TEAL="#2E6E78"; AMBER="#C57B36"; CREAM="#F5F0E8"
GRID="#DBD5CB"; GOOD="#2E8B57"; BAD="#B5482f"; NAVY="#27384A"
plt.rcParams.update({"font.family":"DejaVu Sans","font.size":11,"axes.edgecolor":SUB,
    "axes.labelcolor":INK,"text.color":INK,"xtick.color":SUB,"ytick.color":SUB,
    "axes.titlecolor":INK,"figure.dpi":150,"savefig.dpi":150})
PAGE=(8.5,11.0)
OUT="/Users/khubair/percolator-perp-liquidity/docs/Perp-Liquidity-Design.pdf"

def page(title, kicker=""):
    fig=plt.figure(figsize=PAGE); fig.patch.set_facecolor("white")
    fig.add_artist(Rectangle((0,0.935),1,0.065,transform=fig.transFigure,color=TEAL,zorder=0))
    fig.add_artist(Rectangle((0,0.930),1,0.006,transform=fig.transFigure,color=AMBER,zorder=0))
    fig.text(0.06,0.962,title,color="white",fontsize=17,fontweight="bold",va="center")
    if kicker: fig.text(0.94,0.962,kicker,color="#cfe6ea",fontsize=10,va="center",ha="right")
    return fig

def foot(fig,n):
    fig.text(0.06,0.028,"Perp liquidity design - draft for feedback",color=SUB,fontsize=8)
    fig.text(0.94,0.028,f"{n}",color=SUB,fontsize=9,ha="right")

def para(fig,x,y,text,width=88,size=11.5,color=INK,lh=0.025,weight="normal"):
    lines=[]
    for blk in text.split("\n"):
        lines += (textwrap.wrap(blk,width=width) if blk.strip() else [""])
    for i,ln in enumerate(lines):
        fig.text(x,y-i*lh,ln,fontsize=size,color=color,fontweight=weight,va="top")
    return y-len(lines)*lh

def point(fig,x,y,head,body,width=80,size=11):
    fig.text(x,y,"•",fontsize=12,color=AMBER,va="top")
    fig.text(x+0.022,y,head,fontsize=size,color=INK,fontweight="bold",va="top")
    return para(fig,x+0.022,y-0.023,body,width=width,size=size,color=SUB,lh=0.023)

def box(ax,cx,cy,w,h,text,fc,tc="white",fs=10,weight="bold"):
    ax.add_patch(FancyBboxPatch((cx-w/2,cy-h/2),w,h,boxstyle="round,pad=0.02,rounding_size=0.12",fc=fc,ec=fc,lw=1.4,zorder=2))
    ax.text(cx,cy,text,ha="center",va="center",color=tc,fontsize=fs,fontweight=weight,zorder=3)

def arrow(ax,x1,y1,x2,y2,c=SUB,lw=1.8):
    ax.add_patch(FancyArrowPatch((x1,y1),(x2,y2),arrowstyle="-|>",mutation_scale=14,color=c,lw=lw,zorder=1))

pages=[]

# ---- 1 TITLE ----
fig=plt.figure(figsize=PAGE); fig.patch.set_facecolor(NAVY)
fig.add_artist(Rectangle((0,0),1,1,transform=fig.transFigure,color=NAVY))
fig.add_artist(Rectangle((0,0.64),1,0.01,transform=fig.transFigure,color=AMBER))
fig.text(0.5,0.72,"Permissionless Perps for Any Token",ha="center",color="white",fontsize=27,fontweight="bold")
fig.text(0.5,0.565,"Letting a market make itself,\ninstead of paying for liquidity",ha="center",color="#cfe6ea",fontsize=15,linespacing=1.6)
fig.text(0.5,0.30,"A working design, shared for feedback.",ha="center",color="#9fc7cd",fontsize=12,style="italic")
fig.text(0.5,0.085,"2026",ha="center",color="#7f95a3",fontsize=10)
pages.append(fig)

# ---- 2 THE PROBLEM ----
fig=page("The problem","why new tokens can't have perps")
y=0.87
y=para(fig,0.06,y,"Opening a tradable futures market on a brand new token is hard for one stubborn reason. "
 "Someone has to stand on the other side of every trade. Big venues solve that by paying professional "
 "market makers or seeding a deep liquidity pool. A small project launching its own coin usually can't "
 "afford either.")
y-=0.018
y=para(fig,0.06,y,"Without that depth the market is thin. Prices are easy to push around, fills are poor, and in the "
 "worst case the pool that's backing the trades gets drained by someone gaming it. So most new tokens "
 "simply never get a real perp market, and the ones that try often blow up.")
y-=0.018
y=para(fig,0.06,y,"Memecoins got past this on the spot side with bonding curves. The curve holds everyone's money and "
 "can always pay out, so a token can launch and trade with almost nothing seeded. A perpetual is harder, "
 "because it is a leveraged bet. When a trader wins, real money has to be there to pay them.")
y-=0.022
fig.text(0.06,y,"The question this design answers:",fontsize=12,color=TEAL,fontweight="bold",va="top"); y-=0.03
para(fig,0.06,y,"can a new project open a safe, tradable perp on its token without buying deep liquidity it doesn't have?",
     width=86,size=12.5,color=INK,weight="bold")
foot(fig,2); pages.append(fig)

# ---- 3 THE IDEA ----
fig=page("The idea","let the market make itself")
y=0.87
y=para(fig,0.06,y,"Instead of hiring market makers, an automated pricing curve quotes both sides from the very first "
 "block and takes the other side of trades itself. There is no order book to fill and no one to pay to sit "
 "there quoting.")
y-=0.018
y=para(fig,0.06,y,"The person launching the market puts up a small amount of their own money as a first loss buffer. "
 "That buffer, not a deep pool, is what stands behind the trades. The market starts small and safe, and it "
 "grows on its own as trading fees accumulate. Other people can add to the buffer later if they want it to "
 "support more size.")
y-=0.018
y=para(fig,0.06,y,"In plain terms, this turns the deal from \"pay ten thousand dollars for liquidity you don't have\" into "
 "\"put up a small, refundable stake and let real demand grow the market.\"")
y-=0.022
fig.text(0.06,y,"Two parts do two jobs:",fontsize=12,color=TEAL,fontweight="bold",va="top"); y-=0.032
y=point(fig,0.065,y,"The curve makes it tradable.","An internal price and an automated quote let anyone trade instantly, with no market maker.")
y-=0.006
point(fig,0.065,y,"The buffer makes it solvent.","A small creator stake, growing with fees, is the real money behind the trades. The curve does not create that out of thin air.")
foot(fig,3); pages.append(fig)

# ---- 4 HOW IT WORKS ----
fig=page("How it works","an order, start to finish")
ax=fig.add_axes([0.05,0.12,0.9,0.74]); ax.set_xlim(0,10); ax.set_ylim(0,10); ax.axis("off")
box(ax,1.6,8.7,2.4,1.0,"A trader\narrives",AMBER)
box(ax,5.0,8.7,2.6,1.0,"The curve quotes\na price",TEAL)
box(ax,8.3,8.7,2.6,1.0,"They trade against\nthe creator-backed\nvault",NAVY,fs=9)
arrow(ax,2.8,8.7,3.7,8.7); arrow(ax,6.3,8.7,7.0,8.7)
box(ax,5.0,5.8,7.2,1.1,"The price comes from the token's own recent trading,\nnot an outside feed",  "#5b6b54",fs=10)
arrow(ax,8.3,8.2,5.6,6.35); arrow(ax,5.0,8.2,5.0,6.35)
box(ax,2.3,3.2,3.3,1.2,"A fee that rises\nwith volatility\n(cheap when calm)",AMBER,fs=9.5)
box(ax,7.0,3.2,3.6,1.2,"A size cap tied to\nthe buffer, so it can\nalways cover the risk",BAD,fs=9.5)
arrow(ax,5.0,5.25,2.3,3.8,c=GRID); arrow(ax,5.0,5.25,7.0,3.8,c=GRID)
box(ax,5.0,1.05,7.9,1.4,"Settle and pay out.\nLosing margin pays the winners,\nthe buffer covers any gap.","#6a4a8a",fs=9.5)
for cx in (2.3,7.0): arrow(ax,cx,2.6,5.0,1.75,c=GRID)
fig.text(0.5,0.07,"The curve only sets the price. The price, the fee, the cap, and the payouts all live in the protocol.",
         ha="center",fontsize=9.5,color=SUB,style="italic")
foot(fig,4); pages.append(fig)

# ---- 5 WILL IT WORK ----
fig=page("Will it work?","tradable, and competitive")
y=0.88
y=para(fig,0.06,y,"Two things have to be true. People need to be able to trade from day one without a market maker, "
 "and the curve has to price well enough that it isn't picked off by sharper traders.")
y-=0.014
y=para(fig,0.06,y,"The first is what the automated curve is for. The second comes down to the fee. It adapts to how "
 "volatile the token is: lower when things are calm, to attract real trading, and higher when things are "
 "wild, to protect the buffer. A fixed fee can't do that, and it's the part of the design I'm most "
 "confident in.")
ax=fig.add_axes([0.16,0.30,0.7,0.30])
v=np.linspace(0,1,100); feec=20+55*v+30*v*v
ax.plot(v,feec,color=TEAL,lw=2.6)
ax.axhline(65,color=SUB,ls="--",lw=1.3); ax.text(0.02,69,"a fixed fee (one size for everything)",color=SUB,fontsize=9)
ax.text(0.62,40,"the adaptive fee",color=TEAL,fontsize=10,fontweight="bold")
ax.set_xlabel("how volatile the token is  ->"); ax.set_ylabel("fee charged"); ax.set_yticks([]); ax.set_xticks([])
ax.set_xlim(0,1); ax.set_ylim(0,120); ax.grid(True,color=GRID,lw=0.5)
para(fig,0.06,0.235,"In testing against a plain fixed fee across many simulated markets, the adaptive version came out a "
 "few percent ahead while staying competitive on price. It charges a little more only where the risk "
 "actually is, which is also what makes it hard to game.",width=88,size=10.5,color=SUB,lh=0.023)
foot(fig,5); pages.append(fig)

# ---- 6 IS IT SAFE ----
fig=page("Is it safe?","two promises, stated honestly")
y=0.88
y=point(fig,0.06,y,"The buffer can't be quietly drained.","While anyone has a position open, no one can pull the backing out, and the market can never owe more than the money actually sitting in it.",width=82)
y-=0.008
y=point(fig,0.06,y,"Losses are capped.","The most a market can lose is the capital posted to it, and the creator's own stake is first in line. It can't take down the rest of the platform.",width=82)
y-=0.014
fig.text(0.06,y,"What a trader actually experiences:",fontsize=12,color=TEAL,fontweight="bold",va="top"); y-=0.03
y=para(fig,0.06,y,"In normal conditions a winner closes and takes their full profit, paid by whoever was on the losing "
 "side. The only time a winner gets less is an extreme event where the winning claims are larger than the "
 "whole buffer. Then everyone is paid the same proportional share, it's visible up front, and it is never a "
 "freeze and never zero. That is the same way insurance pools work on every major venue.")
ax=fig.add_axes([0.18,0.16,0.64,0.22])
ax.bar(["money posted\nto the market","most it can\never lose"],[100,80],color=[TEAL,BAD],width=0.55)
ax.set_ylim(0,120); ax.set_yticks([]); ax.grid(True,axis="y",color=GRID,lw=0.5)
ax.set_title("the loss is always smaller than the money behind it",fontsize=10,color=SUB)
foot(fig,6); pages.append(fig)

# ---- 7 CAPITAL MODEL ----
fig=page("Who backs it, and who's at risk","the capital model")
ax=fig.add_axes([0.08,0.16,0.84,0.72]); ax.set_xlim(0,10); ax.set_ylim(0,10); ax.axis("off")
rows=[("The creator's stake","put up at launch; first to absorb any loss","#7a3b2e"),
      ("Extra backing from others","optional; lets the market support more size, earns a share of fees","#b06a36"),
      ("Fees the market has earned","build up over time into a cushion","#c9a227"),
      ("A per-market safety fund","a last-resort backstop, kept separate per market","#2E6E78")]
ytop=9.0; h=1.7
for i,(t,d,c) in enumerate(rows):
    yy=ytop-i*(h+0.18)
    ax.add_patch(FancyBboxPatch((1.2,yy-h),6.7,h,boxstyle="round,pad=0.02,rounding_size=0.1",fc=c,ec=c))
    ax.text(1.5,yy-h/2+0.25,t,color="white",fontsize=11.5,fontweight="bold",va="center")
    ax.text(1.5,yy-h/2-0.32,d,color="#f3ece2",fontsize=9.5,va="center")
arrow(ax,8.5,8.9,8.5,1.5,c=BAD,lw=2.2); ax.text(8.8,5.2,"losses hit top first",color=BAD,rotation=90,va="center",fontsize=9.5,fontweight="bold")
fig.text(0.5,0.085,"The creator underwrites the first loss, so they have skin in the game and won't launch\n"
 "junk markets. Everyone else is protected behind them.",ha="center",fontsize=10,color=SUB,linespacing=1.55)
foot(fig,7); pages.append(fig)

# ---- 8 TWO WAYS TO LAUNCH ----
fig=page("Two ways to launch","pick by how mature the token is")
ax=fig.add_axes([0.05,0.10,0.9,0.78]); ax.set_xlim(0,10); ax.set_ylim(0,10); ax.axis("off")
def card(x,title,color,lines):
    ax.add_patch(FancyBboxPatch((x,1.2),4.3,7.6,boxstyle="round,pad=0.05,rounding_size=0.2",fc="white",ec=color,lw=2.2))
    ax.add_patch(FancyBboxPatch((x,7.8),4.3,1.0,boxstyle="round,pad=0.05,rounding_size=0.2",fc=color,ec=color))
    ax.text(x+2.15,8.3,title,color="white",fontsize=12.5,fontweight="bold",ha="center",va="center")
    yy=7.0
    for h,b in lines:
        ax.text(x+0.3,yy,h,fontsize=10,color=INK,fontweight="bold",va="top")
        wr=textwrap.wrap(b,36)
        for j,ln in enumerate(wr): ax.text(x+0.3,yy-0.4-j*0.33,ln,fontsize=8.8,color=SUB,va="top")
        yy-=0.4+0.33*len(wr)+0.32
card(0.5,"A brand new token",TEAL,[
    ("Price","comes from the token's own trading"),
    ("Safety","payouts capped per period, kept small"),
    ("Best for","fresh launches, hype and social coins"),
    ("Feel","small but real, grows with interest")])
card(5.2,"A token with a real price",AMBER,[
    ("Price","can use a trusted outside feed"),
    ("Safety","wider limits as the market proves out"),
    ("Best for","tokens that already trade somewhere deep"),
    ("Feel","closer to a normal perp over time")])
fig.text(0.5,0.05,"A market can start in the first mode and graduate to the second as it gets real volume and a "
 "reliable price.",ha="center",fontsize=10,color=SUB,style="italic")
foot(fig,8); pages.append(fig)

# ---- 9 BENEFITS ----
fig=page("What's good about it","the upside")
y=0.87
for h,b in [
 ("Anyone can open a market.","No gatekeeper, no listing deal. A project can launch a perp on its own token in minutes."),
 ("No deep liquidity needed.","A small first-loss stake replaces a paid market maker. The market funds its own growth from fees."),
 ("It can't be quietly drained.","The backing is locked while trades are open, and a market can never owe more than it holds."),
 ("Manipulation is bounded.","Pushing the price around is expensive and, in the worst case, the damage is capped at the posted capital."),
 ("Honest payouts.","Winners are paid in full in normal conditions, and the rare exception is a clear, proportional share, not a surprise."),
 ("It grows with success.","More trading means more fees means a bigger, deeper market, with no one having to pre-fund it."),
]:
    y=point(fig,0.06,y,h,b,width=84); y-=0.012
foot(fig,9); pages.append(fig)

# ---- 10 CAVEATS ----
fig=page("Where it's not done yet","the honest caveats")
y=0.885
y=para(fig,0.06,y,"This is a design I want feedback on, so here is where it is genuinely unfinished. None of these are "
 "dealbreakers, but they are the things I would not want to launch without resolving.",size=11,lh=0.024)
y-=0.014
for h,b in [
 ("The loss cap depends on a few pieces being in place.","A hard limit on how big a market can get for its buffer, the backing locked while positions are open, each market's safety fund kept separate, and the creator's stake spent before anyone else's. Without all of them, a fast crash or a determined attacker can push losses onto backers beyond the creator's stake."),
 ("Liquidation has to keep up under stress.","In a fast move with many positions at once, the system has to close them quickly enough. That is an operational problem as much as a design one."),
 ("The young market's price is easy to move.","While a market is thin, its internal price can be pushed around cheaply, so other protocols should not rely on it as a price feed until it is deep."),
 ("\"Always paid in full, no matter what\" isn't free.","Guaranteeing every winner an unlimited payout requires the payout capped and fully funded up front, which costs real capital. The cheap version trades a rare, capped haircut for that."),
]:
    y=point(fig,0.06,y,h,b,width=84,size=10.5); y-=0.01
foot(fig,10); pages.append(fig)

# ---- 11 PATH + FEEDBACK ----
fig=page("The path, and what I'd love feedback on","where it goes")
ax=fig.add_axes([0.05,0.45,0.9,0.42]); ax.set_xlim(0,10); ax.set_ylim(0,10); ax.axis("off")
steps=[("1","Launch small and safe","a capped market anyone can open, backed by a small stake","#2E6E78"),
       ("2","Add the cap and balancing","limit size to the buffer and let the book balance itself, so a small buffer goes further","#9a7d2e"),
       ("3","Cover both sides, grow on fees","back both directions and let the market deepen from its own volume","#7a3b2e"),
       ("4","Optional: fully guaranteed payouts","for markets that want it, pre-fund the cap so every winner is always paid in full","#6a4a8a")]
y0=9.2
for i,(n,t,d,c) in enumerate(steps):
    yy=y0-i*2.35
    ax.add_patch(FancyBboxPatch((0.3,yy-1.9),1.2,1.9,boxstyle="round,pad=0.03,rounding_size=0.12",fc=c,ec=c))
    ax.text(0.9,yy-0.95,n,color="white",fontsize=17,fontweight="bold",ha="center",va="center")
    ax.add_patch(FancyBboxPatch((1.8,yy-1.9),7.8,1.9,boxstyle="round,pad=0.03,rounding_size=0.06",fc="white",ec=c,lw=1.5))
    ax.text(2.05,yy-0.45,t,color=c,fontsize=11,fontweight="bold",va="top")
    for j,ln in enumerate(textwrap.wrap(d,78)): ax.text(2.05,yy-0.95-j*0.34,ln,color=SUB,fontsize=9.3,va="top")
    if i<3: arrow(ax,0.9,yy-1.95,0.9,yy-2.35,c=c,lw=1.8)
fig.text(0.06,0.40,"What I'd love your read on:",fontsize=12,color=TEAL,fontweight="bold")
para(fig,0.06,0.365,"Is a small first-loss stake the right trade against deep liquidity? For a brand new token, is a "
 "capped payout acceptable to traders, or do they expect uncapped upside? Is the liquidation timing risk "
 "manageable in practice? And what am I missing on the safety side?",width=90,size=11,color=SUB,lh=0.025)
foot(fig,11); pages.append(fig)

with PdfPages(OUT) as pdf:
    import os; os.makedirs("/tmp/fb_png",exist_ok=True)
    for i,f in enumerate(pages,1):
        pdf.savefig(f,facecolor=f.get_facecolor()); f.savefig(f"/tmp/fb_png/p{i:02d}.png",facecolor=f.get_facecolor(),dpi=110); plt.close(f)
print("WROTE",OUT,"pages:",len(pages))
