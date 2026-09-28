// minimal charset-loop repro
export function checkSym(): string {
  const symbol = near.jsonGetStr("symbol") ?? "";
  let i = 0;
  while (i < strLength(symbol)) {
    const c = strSlice(symbol, i, i + 1);
    const isDigit = c == "0" || c == "1" || c == "2" || c == "3" || c == "4" || c == "5" || c == "6" || c == "7" || c == "8" || c == "9";
    const isLetter = c == "a" || c == "b" || c == "c" || c == "d" || c == "e" || c == "f" || c == "g" || c == "h" || c == "i" || c == "j" || c == "k" || c == "l" || c == "m" || c == "n" || c == "o" || c == "p" || c == "q" || c == "r" || c == "s" || c == "t" || c == "u" || c == "v" || c == "w" || c == "x" || c == "y" || c == "z";
    if (!isLetter && !isDigit) { return "BAD:" + c; }
    i = i + 1;
  }
  return "OK:" + symbol;
}

export function checkNeg(): string {
  // isolate: ! on a comparison-chain local
  const c = near.jsonGetStr("c") ?? "a";
  const isLetter = c == "a" || c == "b" || c == "z";
  if (!isLetter) { return "NOT-LETTER"; }
  return "LETTER";
}

export function checkOr(): string {
  // isolate: || chain result used directly in if
  const c = near.jsonGetStr("c") ?? "a";
  if (c == "a" || c == "b") { return "MATCH"; }
  return "NO";
}

export function checkLocalPos(): string {
  // || chain bound to a local, used POSITIVELY (no negation)
  const c = near.jsonGetStr("c") ?? "a";
  const isLetter = c == "a" || c == "b" || c == "z";
  if (isLetter) { return "LETTER"; }
  return "NOT-LETTER";
}

export function checkLocalNum(): string {
  // numeric local negated — is ! broken on locals in general?
  const n = strToNum(near.jsonGetStr("n") ?? "0");
  if (!n) { return "ZERO"; }
  return "NONZERO";
}

export function checkCmpNeg(): string {
  // direct comparison (no ||) bound to local, negated
  const c = near.jsonGetStr("c") ?? "a";
  const isA = c == "a";
  if (!isA) { return "NOT-A"; }
  return "A";
}

export function notEmpty(): string {
  // !"" must be TRUE (JS falsy set includes the empty string)
  return !"" ? "TRUE" : "FALSE";
}

export function notLetterA(): string {
  // !"a" must be FALSE
  return !"a" ? "TRUE" : "FALSE";
}

export function notZero(): string {
  // !0 must be TRUE (Num 0 falsy)
  return !0 ? "TRUE" : "FALSE";
}

export function notFive(): string {
  // !5 must be FALSE — guard: naive strLength-based lowering would be
  // true here (5 >> 32 == 0 with no type check)
  return !5 ? "TRUE" : "FALSE";
}

export function notNull(): string {
  // !null must be TRUE (Nil falsy)
  return !null ? "TRUE" : "FALSE";
}

export function notTrue(): string {
  // !true must be FALSE
  return !true ? "TRUE" : "FALSE";
}

export function notIdentEmpty(): string {
  // the real-world shape: identifier holding "" negated
  const s = near.jsonGetStr("s") ?? "";
  return !s ? "TRUE" : "FALSE";
}

export function notDoubleNeg(): string {
  // nested !! on the same name — let-shadowing must keep this at JS truth
  const s = near.jsonGetStr("s") ?? "";
  return !!s ? "NONEMPTY" : "EMPTY";
}

