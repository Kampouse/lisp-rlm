// ─── clmm_v5_driver.ts — node CLI for the byte-parity harness ───
//
// Reads an ops JSON on stdin, applies each op to the twin Pool, and prints
// one line per op:  RET:<return>   STATE:<get_state string>
// The python harness byte-diffs these lines against near-mock + real wasm.
//
// RET lines are POST-RESOLVE "used:<kept>" — ft2's resolve maps the pool's
// raw return r to used = (r empty ? amt : amt − r); seeds/mint legs return
// "0" (keep all), swap legs return the refund remainder.
import { Pool } from "./clmm_v5.ts";

interface Op {
  op: string;
  toka?: string; tokb?: string; fee_shift?: string; s0?: string;
  tok?: string; amt?: string;
  d?: string; lo?: number; hi?: number; rx?: string; ry?: string;
}

function used(raw: string, amt: string): string {
  return raw === "" ? amt : (BigInt(amt) - BigInt(raw)).toString();
}

async function main(): Promise<void> {
  const chunks: Buffer[] = [];
  for await (const c of process.stdin) chunks.push(c as Buffer);
  const { ops } = JSON.parse(Buffer.concat(chunks).toString()) as { ops: Op[] };
  const p = new Pool();
  for (const o of ops) {
    let ret: string;
    switch (o.op) {
      case "init":
        ret = p.init({ toka: o.toka!, tokb: o.tokb!, fee_shift: o.fee_shift!, s0: o.s0! });
        break;
      case "seed": {
        const raw = p.seed(o.tok as "x" | "y", o.amt!);
        ret = "used:" + used(raw, o.amt!);
        break;
      }
      case "mint": {
        const [rx, ry] = p.mintPos(o.d!, o.lo!, o.hi!, o.rx!, o.ry!);
        ret = "used:" + rx + ",used:" + ry;
        break;
      }
      case "swap": {
        const raw = o.tok === "x" ? p.swapX(o.amt!) : p.swapY(o.amt!);
        ret = "used:" + used(raw, o.amt!);
        break;
      }
      case "req":
        ret = p.positionRequirements(o.d!, o.lo!, o.hi!);
        break;
      case "quote":
        ret = p.quote(o.amt!);
        break;
      default:
        ret = "unknown-op";
    }
    process.stdout.write("RET:" + ret + "\n");
    process.stdout.write("STATE:" + p.getState() + "\n");
  }
}
void main();
