function muldiv(a: string, b: string, c: string): string { return u128Div(u128Mul(a, b), c); }
const Q32: string = "4294967296";
export function w1(): string { let r: string = u128Div(muldiv(Q32, "4295397017", Q32), "1"); return r; }
