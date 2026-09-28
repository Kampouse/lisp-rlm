// ─── On-chain Q-learning (fixed-point, integer-only IR) ───
// 6-state corridor s0..s5, 3 arms L/C/R. Q stored ×100 ("centi-Q") as
// strings in contract storage. eps is an integer percent (0..100).
// Reward: forward +1, bump wall s5 −1, idle 0. Update: Q += (r·100 − Q)/2.

export function initQ(): number {
  const arms: string[] = ["L", "C", "R"];
  let i = 0;
  while (i < 3) {
    let j = 0;
    while (j < 6) {
      near.storageSet(j + "|" + arms[i], "50");
      j = j + 1;
    }
    i = i + 1;
  }
  near.storageSet("rng", "1");
  return 18;
}

export function qget(k: string): number {
  return strToNum(near.storageGet(k) ?? "50");
}

// ε-greedy act + online update. Returns "key:reward:newState".
export function act(s: number, eps: number): string {
  const keys: string[] = ["L", "C", "R"];
  // 1) greedy arm: max Q[s|*]
  let best = "L";
  let bv = -1;
  let i = 0;
  while (i < 3) {
    const v = strToNum(near.storageGet(s + "|" + keys[i]) ?? "50");
    if (v > bv) {
      bv = v;
      best = keys[i];
    }
    i = i + 1;
  }
  // 2) deterministic on-chain RNG counter → explore with prob eps%
  const n = strToNum(near.storageGet("rng") ?? "0") + 1;
  near.storageSet("rng", toStr(n));
  const roll = (n * 7) % 100;
  let choice = best;
  if (roll < eps) {
    choice = keys[n % 3];
  }
  // 3) environment: L advances (+1) until wall s5 (−1); others idle (0)
  let r = 0;
  let s2 = s;
  if (choice == "L") {
    if (s < 5) {
      r = 1;
      s2 = s + 1;
    } else {
      r = -1;
    }
  }
  // 4) Q-update in centi-units: Q += (r·100 − Q) / 2
  const k = s + "|" + choice;
  const q = strToNum(near.storageGet(k) ?? "50");
  const nq = q + ((r * 100 - q) / 2);
  near.storageSet(k, toStr(nq));
  return k + ":" + toStr(r) + ":" + toStr(s2);
}

// Show the whole table as one JSON-ish doc
export function qtable(): string {
  const arms: string[] = ["L", "C", "R"];
  let out = "{";
  let s = 0;
  while (s < 6) {
    let i = 0;
    while (i < 3) {
      const k = s + "|" + arms[i];
      out = out + k + ":" + (near.storageGet(k) ?? "?");
      if (s != 5 || i != 2) {
        out = out + ",";
      }
      i = i + 1;
    }
    s = s + 1;
  }
  return out + "}";
}
