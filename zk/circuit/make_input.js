// Client-side flow: compute commitments, generate witness, prove, convert.
const { buildPoseidonOpt } = require("circomlibjs");
const fs = require("fs");

async function main() {
  const poseidon = await buildPoseidonOpt();
  const F = poseidon.F;

  const x = "100";
  const y = "50";
  const sx = "1234567890123456789012345678901234567890123456789012345678901234567890";
  const sy = "9876543210987654321098765432109876543210987654321098765432109876543210";

  const Cx = F.toString(poseidon([x, sx]));
  const Cy = F.toString(poseidon([y, sy]));
  console.log("Cx =", Cx);
  console.log("Cy =", Cy);

  fs.writeFileSync("input.json", JSON.stringify({ x, y, sx, sy, Cx, Cy }, null, 2));
  console.log("input.json written");
}

main().catch((e) => { console.error(e); process.exit(1); });
