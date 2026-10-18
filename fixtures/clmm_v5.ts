// ─── clmm_v5.ts — CLMM v5 TS frontend twin (byte-parity harness side) ───
//
// BigInt port of lib/clmm_pool_v5.lisp's observable semantics, op-for-op:
//   p5-G (tick → sqrt price, fixed-point square-and-multiply + isqrt),
//   p5-dy/dyi/dx/dxo, p5-sfdx/sfdyi/tstar, p5-swarm-up/dn,
//   p5-swap-x/y + p5-leg-decide (post-b535cae4 escrow semantics),
//   seeds, mint legs (escrow + LQ cells), position_requirements.
//
// Wire format pinned to the live contract (pool3.clmm5-51fe5a.testnet):
//   get_state = "S:..:P:..:X:..:Y:..:IMIN:..:IMAX:..:LAST:..:ESCX:..:ESCY:
//                ..:TOKX:..:TOKY:..:SHIFT:..:FEEX:..:FEEY:.."
//   LAST      = "swap-out:<out>:rem:<rem>"   (never re-derived, only parsed)
//   ft legs   = "used:<kept>"                (ft2 resolve: refund = amt−used)
//
// Escrow law (b535cae4): commit releases the REFUND slice (esc −= rem); the
// consumed slice stays escrowed. Payout failure nets the ledger to zero.
//
// Not modeled (invisible in get_state): PM two-leg dance (mintPos is the
// X-then-Y pair), per-cell fee snapshots (6|FS/6|FG — only FEEX/FEEY
// accumulators are state-visible), POS/OWED ledgers.
//
// Edit this file so it mirrors lib/clmm_pool_v5.lisp — never the reverse.

export class Pool {
  s = new Map<string, string>();
  // G(i) memo — isqrt of huge powers is the hot path
  private gcache = new Map<number, bigint>();

  bz(k: string): string { return this.s.get(k) ?? "0"; }
  put(k: string, v: string): void { this.s.set(k, v); }
  li_add(a: string, b: string): string { return (BigInt(a) + BigInt(b)).toString(); }
  li_sub(a: string, b: string): string { return (BigInt(a) - BigInt(b)).toString(); }
  li_cmp(a: string, b: string): number {
    const x = BigInt(a), y = BigInt(b);
    return x < y ? -1 : x > y ? 1 : 0;
  }

  // ── p5-shf / p5-shr8: fee shift (default 8) ──
  shift(): bigint { const sh = this.bz("6|SHIFT"); return BigInt(sh === "0" ? "8" : sh); }
  shr8(v: string): string { return (BigInt(v) >> this.shift()).toString(); }

  // ── isqrt (floor) on BigInt ──
  static isqrt(n: bigint): bigint {
    if (n < 2n) return n;
    let x = 1n << (BigInt(n.toString(2).length + 1) / 2n);
    for (;;) {
      const y = (x + n / x) >> 1n;
      if (y >= x) return x;
      x = y;
    }
  }

  // ── p5-G: S(i) = isqrt(1.0001^i · 2^128) ──
  // fixed-point scale 10^108; Bp = 10001·10^104, Bn = ⌊10^112/10001⌋;
  // square-and-multiply with per-step floor (⌊a·b/10^108⌋), then
  // N = ⌊acc·2^128/10^108⌋, isqrt. i=0 → 2^64.
  G(i: number): bigint {
    const hit = this.gcache.get(i);
    if (hit !== undefined) return hit;
    const SC = 10n ** 108n;
    const Bp = 10001n * 10n ** 104n;
    const Bn = 10n ** 112n / 10001n;
    let acc = SC;
    let R = i < 0 ? Bn : Bp;
    let q = Math.abs(i);
    while (q > 0) {
      if (q % 2 === 1) acc = (acc * R) / SC;
      q = Math.floor(q / 2);
      if (q > 0) R = (R * R) / SC;
    }
    const N = (acc << 128n) / SC;
    const r = Pool.isqrt(N);
    this.gcache.set(i, r);
    return r;
  }
  Gs(i: number): string { return this.G(i).toString(); }

  // ── p5-dy/dyi/dx/dxo (BigInt mirrors of the kernel ops) ──
  dy(L: string, S: string, S2: string): string {
    // L·(S2−S) >> 64 ; requires S2 ≥ S
    return ((BigInt(L) * (BigInt(S2) - BigInt(S))) >> 64n).toString();
  }
  dyi(L: string, S: string, S2: string): string { return this.dy(L, S2, S); }
  dx(L: string, S: string, S2: string): string {
    // L·(S2−S) // (S·S2) ; requires S2 ≥ S
    const a = BigInt(S2) - BigInt(S);
    return ((BigInt(L) * a) / (BigInt(S) * BigInt(S2))).toString();
  }
  dxo(L: string, S: string, S2: string): string { return this.dx(L, S2, S); }

  // ── p5-sfdx: smallest S2 ≥ S with dx_in ≤ dx ──
  // L=0 → 2^127 ; else S2 = min(2^127, max(S, ceil((L·S+1)/(L+(dx+1)·S))))
  sfdx(L: string, S: string, dx: string): string {
    const C127 = 340282366920938463463374607431768211456n;
    if (BigInt(L) <= 0n) return C127.toString();
    const tB = (BigInt(dx) + 1n) * BigInt(S) + BigInt(L);
    const tC = BigInt(L) * BigInt(S) + 1n;
    let q = tC / tB;
    if (tC % tB > 0n) q += 1n;
    let tE = q;
    if (tE > C127) tE = C127;
    if (tE < BigInt(S)) tE = BigInt(S);
    return tE.toString();
  }

  // ── p5-sfdyi via tstar: smallest S2 ≤ S with dy_in ≤ dy ──
  // L=0 → 1 ; else max(1, S − ⌊(2^64·(dy+1)−1)/L⌋)
  sfdyi(L: string, S: string, dy: string): string {
    if (BigInt(L) <= 0n) return "1";
    const t = ((1n << 64n) * (BigInt(dy) + 1n) - 1n) / BigInt(L);
    const r = BigInt(S) - t;
    return (r < 1n ? 1n : r).toString();
  }

  // ── p5-swarm-up: sell X for Y, walk P up. Returns [out, rem, committed] ──
  swarmUp(s0: string, p0: bigint, xbook: string, ybook: string, amt: string): [string, string, boolean] {
    const F = BigInt(this.shr8(amt));
    const v = BigInt(amt) - F;
    const imax = BigInt(this.bz("6|IMAX"));
    const gmax = this.G(Number(imax) + 1);
    let pp = p0, ss = BigInt(s0), acc = 0n, rm = v;
    const v0 = v;
    let moved = false;
    for (;;) {
      // lisp predicates: write P/S iff NOT (acc==0 && rm==v0); commit iff
      // rm != v0 (the outer `if (= rmf v)` guard) — a boundary exit after
      // partial consumption COMMITS and refunds the rest
      const done = (): [string, string, boolean] => {
        if (!(acc === 0n && rm === v0)) {
          this.put("6|P", pp.toString());
          this.put("6|S", ss.toString());
        }
        return [acc.toString(), rm.toString(), rm !== v0];
      };
      if (rm === 0n) return done();
      if (ss >= gmax || pp >= imax + 1n) return done();
      const L = this.bz("6|LQ:" + pp);
      const gh = this.G(Number(pp) + 1);
      const dy = BigInt(this.dy(L, ss.toString(), gh.toString()));
      const dx = BigInt(this.dx(L, ss.toString(), gh.toString()));
      if (dx <= rm) {
        acc += dy === 0n ? 0n : dy;
        rm -= dx;
        pp += 1n; ss = gh; moved = true;
        continue;
      }
      const s2 = BigInt(this.sfdx(L, ss.toString(), rm.toString()));
      this.put("6|P", pp.toString());
      this.put("6|S", s2.toString());
      const out = acc + BigInt(this.dy(L, ss.toString(), s2.toString()));
      return [out.toString(), "0", true];
    }
  }

  // ── p5-swarm-dn: sell Y for X, walk P down ──
  swarmDn(s0: string, p0: bigint, xbook: string, ybook: string, amt: string): [string, string, boolean] {
    const F = BigInt(this.shr8(amt));
    const v = BigInt(amt) - F;
    const imin = BigInt(this.bz("6|IMIN"));
    const gmin = this.G(Number(imin));
    let pp = p0, ss = BigInt(s0), acc = 0n, rm = v;
    const v0 = v;
    let moved = false;
    for (;;) {
      // mirror of the up-arm exit predicates (see swarmUp)
      const done = (): [string, string, boolean] => {
        if (!(acc === 0n && rm === v0)) {
          this.put("6|P", pp.toString());
          this.put("6|S", ss.toString());
        }
        return [acc.toString(), rm.toString(), rm !== v0];
      };
      if (rm === 0n) return done();
      if (ss <= gmin || pp <= imin) return done();
      const L = this.bz("6|LQ:" + pp);
      const gl = this.G(Number(pp) - 1);
      const dxo = BigInt(this.dxo(L, ss.toString(), gl.toString()));
      const dyi = BigInt(this.dyi(L, ss.toString(), gl.toString()));
      if (dyi <= rm) {
        acc += dyi === 0n ? 0n : dxo;
        rm -= dyi;
        pp -= 1n; ss = gl; moved = true;
        continue;
      }
      const s2 = BigInt(this.sfdyi(L, ss.toString(), rm.toString()));
      this.put("6|P", pp.toString());
      this.put("6|S", s2.toString());
      const out = acc + BigInt(this.dxo(L, ss.toString(), s2.toString()));
      return [out.toString(), "0", true];
    }
  }

  // ── verbs ──
  init(args: { toka: string; tokb: string; fee_shift: string; s0: string }): string {
    this.put("6|TOKX", args.toka);
    this.put("6|TOKY", args.tokb);
    this.put("6|S", args.s0);
    this.put("6|P", "0");
    this.put("6|IMIN", "0");
    this.put("6|IMAX", "0");
    this.put("6|X", "0");
    this.put("6|Y", "0");
    this.put("6|ESCX", "0");
    this.put("6|ESCY", "0");
    this.put("6|SHIFT", args.fee_shift);
    this.put("6|FEEX", "0");
    this.put("6|FEEY", "0");
    this.put("6|LAST", "0");
    return "init-ok";
  }

  // on-transfer seed: pure escrow credit, chain result "0" ⇒ ft2 keeps all
  seed(tok: "x" | "y", amt: string): string {
    const k = tok === "x" ? "6|ESCX" : "6|ESCY";
    this.put(k, this.li_add(this.bz(k), amt));
    return "0";
  }

  // position_requirements verb: legacy {"d","a","b"} → "rx:<rx>:ry:<ry>"
  // reqx: Σ dx(d, G(i), G(i+1)); reqy: Σ dy(d, G(i), G(i+1))
  positionRequirements(d: string, lo: number, hi: number): string {
    let sx = BigInt(this.G(lo)); let accx = 0n;
    let sy = BigInt(this.G(lo)); let accy = 0n;
    for (let i = lo; i <= hi; i++) {
      const g1 = this.G(i + 1);
      accx += BigInt(this.dx(d, sx.toString(), g1.toString()));
      accy += BigInt(this.dy(d, sy.toString(), g1.toString()));
      sx = g1; sy = g1;
    }
    return "rx:" + accx.toString() + ":ry:" + accy.toString();
  }

  // mint pair (X leg then Y leg, state-equivalent of the PM dance):
  // each leg credits its escrow and keeps the full attach (used:amt);
  // finish writes LQ:<i> += d over [lo,hi] and widens IMIN/IMAX
  mintPos(d: string, lo: number, hi: number, rx: string, ry: string): [string, string] {
    this.put("6|ESCX", this.li_add(this.bz("6|ESCX"), rx));
    this.put("6|ESCY", this.li_add(this.bz("6|ESCY"), ry));
    for (let i = lo; i <= hi; i++) {
      const k = "6|LQ:" + i;
      this.put(k, this.li_add(this.bz(k), d));
    }
    if (BigInt(lo) < BigInt(this.bz("6|IMIN"))) this.put("6|IMIN", lo.toString());
    if (BigInt(hi) > BigInt(this.bz("6|IMAX"))) this.put("6|IMAX", hi.toString());
    return [rx, ry];
  }

  // ── ft swap legs (X-attach then Y-attach), post-b535cae4 escrow law ──
  // X-attach: escrow_x += amt; F = amt >> shift; walk up on v = amt − F.
  //   committed (rem != v): escrow_x −= rem (refund leaves via resolve);
  //     escrow_y −= out (payout drain); books: X += v − rem, Y += out;
  //     FEEX += F; LAST written; used = amt − rem.
  //   out == 0 degenerate: undo intake book delta + P/S + escrow (−amt);
  //     LAST stays as the arm wrote it ("swap-out:0:rem:0"); used = 0.
  //   not committed (sentinel): undo escrow only; used = 0.
  swapX(amt: string, mo?: string): string {
    if (this.bz("6|TOKX") === "") return amt;
    if (BigInt(amt) === 0n) return amt;
    const snap = mo ? new Map(this.s) : null; // abort ⇒ full revert
    const s0 = this.bz("6|S"), p0 = BigInt(this.bz("6|P"));
    const xbook = this.bz("6|X"), ybook = this.bz("6|Y");
    this.put("6|ESCX", this.li_add(this.bz("6|ESCX"), amt));
    const [out, rem, committed] = this.swarmUp(s0, p0, xbook, ybook, amt);
    const F = BigInt(this.shr8(amt));
    const v = BigInt(amt) - F;
    if (!committed) {
      this.put("6|ESCX", this.li_sub(this.bz("6|ESCX"), amt));
      return amt; // full refund
    }
    const o = BigInt(out);
    if (o === 0n) {
      // degenerate: walker committed with zero output — leg-decide reverts
      this.put("6|ESCX", this.li_sub(this.bz("6|ESCX"), amt));
      this.put("6|X", xbook);
      this.put("6|P", p0.toString());
      this.put("6|S", s0);
      return amt; // full refund
    }
    // min-out floor (msg "mo"): out < mo ⇒ ERR_MO abort — the contract's
    // p5-fail reverts EVERY write (escrow, books, P/S, fee) and ft2's
    // resolve sees an empty result ⇒ full refund incl. fee
    if (snap && BigInt(out) < BigInt(mo!)) {
      this.s = snap;
      return amt;
    }
    // commit (escrow law: release the refund slice, keep the consumed)
    this.put("6|ESCX", this.li_sub(this.bz("6|ESCX"), rem));
    this.put("6|ESCY", this.li_sub(this.bz("6|ESCY"), out));
    this.put("6|X", (BigInt(xbook) + v - BigInt(rem)).toString());
    this.put("6|Y", (BigInt(ybook) + o).toString());
    this.put("6|FEEX", (BigInt(this.bz("6|FEEX")) + F).toString());
    this.put("6|LAST", "swap-out:" + out + ":rem:" + rem);
    return rem; // refund = remainder (resolve: used = amt − rem)
  }

  // Y-attach mirror: swarm-down, out-leg from 6|ESCX, fees → FEEY
  swapY(amt: string, mo?: string): string {
    if (this.bz("6|TOKX") === "") return amt;
    if (BigInt(amt) === 0n) return amt;
    const snap = mo ? new Map(this.s) : null; // abort ⇒ full revert
    const s0 = this.bz("6|S"), p0 = BigInt(this.bz("6|P"));
    const xbook = this.bz("6|X"), ybook = this.bz("6|Y");
    this.put("6|ESCY", this.li_add(this.bz("6|ESCY"), amt));
    const [out, rem, committed] = this.swarmDn(s0, p0, xbook, ybook, amt);
    const F = BigInt(this.shr8(amt));
    const v = BigInt(amt) - F;
    if (!committed) {
      this.put("6|ESCY", this.li_sub(this.bz("6|ESCY"), amt));
      return amt; // full refund
    }
    const o = BigInt(out);
    if (o === 0n) {
      this.put("6|ESCY", this.li_sub(this.bz("6|ESCY"), amt));
      this.put("6|Y", ybook);
      this.put("6|P", p0.toString());
      this.put("6|S", s0);
      return amt; // full refund
    }
    // min-out floor (see swapX)
    if (snap && BigInt(out) < BigInt(mo!)) {
      this.s = snap;
      return amt;
    }
    this.put("6|ESCY", this.li_sub(this.bz("6|ESCY"), rem));
    this.put("6|ESCX", this.li_sub(this.bz("6|ESCX"), out));
    this.put("6|Y", (BigInt(ybook) + v - BigInt(rem)).toString());
    this.put("6|X", (BigInt(xbook) + o).toString());
    this.put("6|FEEY", (BigInt(this.bz("6|FEEY")) + F).toString());
    this.put("6|LAST", "swap-out:" + out + ":rem:" + rem);
    return rem; // refund = remainder
  }

  // ── p5-quote: zero-write preview, NO fee skim (caller pre-skims) ──
  // "q:<out>:rm:<rm>"; negative v (down) strips its sign first
  quote(vSigned: string): string {
    const neg = vSigned.startsWith("-");
    const v = neg ? vSigned.slice(1) : vSigned;
    if (BigInt(v) === 0n) return "q:0:0";
    const s = this.bz("6|S"), p = BigInt(this.bz("6|P"));
    return neg ? this.qarmDn(s, p, v) : this.qarmUp(s, p, v);
  }
  private qarmUp(s: string, p: bigint, v: string): string {
    const imax = BigInt(this.bz("6|IMAX"));
    const gmax = this.G(Number(imax) + 1);
    let q = p, ss = BigInt(s), acc = 0n, rm = BigInt(v);
    for (;;) {
      if (rm === 0n || ss >= gmax || q >= imax + 1n)
        return "q:" + acc.toString() + ":rm:" + rm.toString();
      const L = this.bz("6|LQ:" + q);
      const gh = this.G(Number(q) + 1);
      const dx = BigInt(this.dx(L, ss.toString(), gh.toString()));
      if (dx <= rm) {
        acc += dx === 0n ? 0n : BigInt(this.dy(L, ss.toString(), gh.toString()));
        rm -= dx;
        q += 1n; ss = gh;
        continue;
      }
      const s2 = BigInt(this.sfdx(L, ss.toString(), rm.toString()));
      return "q:" + (acc + BigInt(this.dy(L, ss.toString(), s2.toString()))).toString() + ":rm:0";
    }
  }
  private qarmDn(s: string, p: bigint, v: string): string {
    const imin = BigInt(this.bz("6|IMIN"));
    const gmin = this.G(Number(imin));
    let q = p, ss = BigInt(s), acc = 0n, rm = BigInt(v);
    for (;;) {
      if (rm === 0n || ss <= gmin || q <= imin)
        return "q:" + acc.toString() + ":rm:" + rm.toString();
      const L = this.bz("6|LQ:" + q);
      const gl = this.G(Number(q) - 1);
      const dyi = BigInt(this.dyi(L, ss.toString(), gl.toString()));
      if (dyi <= rm) {
        acc += dyi === 0n ? 0n : BigInt(this.dxo(L, ss.toString(), gl.toString()));
        rm -= dyi;
        q -= 1n; ss = gl;
        continue;
      }
      const s2 = BigInt(this.sfdyi(L, ss.toString(), rm.toString()));
      return "q:" + (acc + BigInt(this.dxo(L, ss.toString(), s2.toString()))).toString() + ":rm:0";
    }
  }

  // ── safe frontend API: the raw wire formats become private details ──

  // preview a real swap of `amt`: fee skim first (the raw quote walks the
  // gross amount), then the walk. out is exact — quote(amt−fee).out equals
  // the executed swap's LAST out.
  quoteSwap(amt: string, dir: "x" | "y"): { out: string; refunded: string; kept: string; fee: string } {
    const fee = BigInt(this.shr8(amt));
    const q = this.quote(dir === "x" ? (BigInt(amt) - fee).toString() : "-" + (BigInt(amt) - fee).toString());
    const [out, rm] = q.slice(2).split(":rm:");
    const kept = BigInt(amt) - BigInt(rm);
    return { out, refunded: rm, kept: kept.toString(), fee: fee.toString() };
  }

  // trap 1: the chain result of a swap leg is "used:<kept>" — kept, NOT
  // spent. Never show `used` as the spent amount.
  static parseSwapResult(amt: string, usedStr: string): { kept: string; refunded: string } {
    const kept = BigInt(usedStr.replace("used:", ""));
    return { kept: kept.toString(), refunded: (BigInt(amt) - kept).toString() };
  }

  // trap 3: get_state enumerates only NONZERO LQ cells — parseState
  // materializes the full zero-filled ladder over [IMIN..IMAX] so callers
  // can never misindex the orderbook by assuming dense or sparse arrays.
  static parseState(s: string): {
    S: string; P: string; X: string; Y: string; imin: number; imax: number;
    last: string; escx: string; escy: string; shift: string; feex: string; feey: string;
    ticks: { tick: number; lq: string }[];
  } {
    const kv = new Map<string, string>();
    const ticks: { tick: number; lq: string }[] = [];
    const re = /(-?\d+)\|(\d+)/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(s)) !== null) ticks.push({ tick: Number(m[1]), lq: m[2] });
    // LAST embeds colons ("swap-out:<out>:rem:<rem>") — extract it first,
    // then pair the remaining colon-delimited K:V fields
    const lastM = /LAST:(swap-out:\d+:rem:\d+|0)(:|$)/.exec(s);
    const last = lastM ? lastM[1] : "0";
    const rest = s.replace(/LAST:(swap-out:\d+:rem:\d+|0)/, "LASTX");
    const parts = rest.split(":");
    for (let i = 0; i + 1 < parts.length; i += 2) {
      if (parts[i] === "LQ") continue; // "<tick>|<lq>" pair — handled above
      kv.set(parts[i], parts[i + 1]);
    }
    const g = (k: string): string => kv.get(k) ?? "";
    const imin = Number(g("IMIN")), imax = Number(g("IMAX"));
    const byTick = new Map(ticks.map(t => [t.tick, t.lq]));
    const ladder: { tick: number; lq: string }[] = [];
    for (let i = imin; i <= imax; i++) ladder.push({ tick: i, lq: byTick.get(i) ?? "0" });
    return {
      S: g("S"), P: g("P"), X: g("X"), Y: g("Y"),
      imin, imax, last, escx: g("ESCX"), escy: g("ESCY"),
      shift: g("SHIFT") || "8", feex: g("FEEX"), feey: g("FEEY"),
      ticks: ladder,
    };
  }

  getState(): string {
    // p5-state: S:P:X:Y:IMIN:IMAX then :LQ:<i>|<v> for every NONZERO cell
    // in [IMIN..IMAX], then :LAST:ESCX:ESCY:TOKX:TOKY:SHIFT:FEEX:FEEY.
    // SHIFT prints "8" when unset/0.
    const parts = [
      "S", this.bz("6|S"),
      "P", this.bz("6|P"),
      "X", this.bz("6|X"),
      "Y", this.bz("6|Y"),
      "IMIN", this.bz("6|IMIN"),
      "IMAX", this.bz("6|IMAX"),
    ];
    let out = parts.join(":");
    const lo = Number(this.bz("6|IMIN"));
    const hi = Number(this.bz("6|IMAX"));
    for (let i = lo; i <= hi; i++) {
      const v = this.bz("6|LQ:" + i);
      if (BigInt(v) !== 0n) out += ":LQ:" + i + "|" + v;
    }
    const sh = this.bz("6|SHIFT");
    out += ":LAST:" + this.bz("6|LAST");
    out += ":ESCX:" + this.bz("6|ESCX");
    out += ":ESCY:" + this.bz("6|ESCY");
    out += ":TOKX:" + this.bz("6|TOKX");
    out += ":TOKY:" + this.bz("6|TOKY");
    out += ":SHIFT:" + (sh === "0" ? "8" : sh);
    out += ":FEEX:" + this.bz("6|FEEX");
    out += ":FEEY:" + this.bz("6|FEEY");
    return out;
  }
}
