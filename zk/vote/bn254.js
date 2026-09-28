// Minimal correct BN254 point arithmetic for generating encrypted ballots
const P = 21888242871839275222246405745257275088696311157297823662689037894645226208583n;

// BN254 G1: y^2 = x^3 + 3
const G = [1n, 2n];

function modInv(a, m = P) {
  // Fermat's little theorem: a^(p-2) mod p
  let result = 1n;
  let base = ((a % m) + m) % m; // handle negatives
  let exp = m - 2n;
  while (exp > 0n) {
    if (exp & 1n) result = (result * base) % m;
    base = (base * base) % m;
    exp >>= 1n;
  }
  return result;
}

function pointAdd(p1, p2) {
  if (p1 === null) return p2;
  if (p2 === null) return p1;
  const [x1, y1] = p1;
  const [x2, y2] = p2;
  // p1 == -p2 (same x, negated y) → infinity
  if (x1 === x2 && (y1 + y2) % P === 0n) return null;
  // p1 == p2 → doubling
  if (x1 === x2 && y1 === y2) return pointDouble(p1);
  // General addition
  const lambda = (((y2 - y1) % P) * modInv((x2 - x1) % P)) % P;
  const x3 = ((lambda * lambda) % P - x1 - x2 + 2n * P) % P;
  const y3 = ((lambda * (x1 - x3)) % P - y1 + P) % P;
  return [x3, y3];
}

function pointDouble(p) {
  if (p === null || p[1] === 0n) return null;
  const [x, y] = p;
  const lambda = ((3n * x * x) % P * modInv((2n * y) % P)) % P;
  const x3 = ((lambda * lambda) % P - 2n * x + P) % P;
  const y3 = ((lambda * (x - x3)) % P - y + P) % P;
  return [x3, y3];
}

function pointMul(point, k) {
  let result = null;
  let addend = point;
  let n = ((k % (P - 1n)) + (P - 1n)) % (P - 1n); // reduce scalar
  while (n > 0n) {
    if (n & 1n) result = pointAdd(result, addend);
    addend = pointDouble(addend);
    n >>= 1n;
  }
  return result;
}

function onCurve(p) {
  if (p === null) return true;
  const [x, y] = p;
  const lhs = (y * y) % P;
  const rhs = (x * x * x + 3n) % P;
  return lhs === rhs;
}

// Encode to LE-halves hex (what the NEAR host expects)
function pointToHex(p) {
  if (p === null) return "0".repeat(128);
  const [x, y] = p;
  const elemLE = (v) => {
    const lo = v & ((1n << 128n) - 1n);
    const hi = v >> 128n;
    // Convert BigInt to little-endian byte array
    const loArr = [];
    let l = lo;
    for (let i = 0; i < 16; i++) { loArr.push(Number(l & 0xffn)); l >>= 8n; }
    const hiArr = [];
    let h = hi;
    for (let i = 0; i < 16; i++) { hiArr.push(Number(h & 0xffn)); h >>= 8n; }
    return [...loArr, ...hiArr].map(b => b.toString(16).padStart(2, "0")).join("");
  };
  return elemLE(x) + elemLE(y);
}

module.exports = { P, G, pointAdd, pointDouble, pointMul, onCurve, pointToHex, modInv };
