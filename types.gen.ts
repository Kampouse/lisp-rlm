// GENERATED from src/ops_spec.rs (ops-spec PoC, TASK-DX-AGREE item 7) — do not edit.
// Regenerate: cargo test --test ops_spec_test (golden-compared, fails on drift).

export interface LispOps {
  "+": (...xs: number[]) => number;
  "-": (...xs: number[]) => number;
  "*": (...xs: number[]) => number;
  "/": (...xs: number[]) => number;
  mod: (a: number, b: number) => number;
  abs: (x: number) => number;
  "wrap-add": (...xs: number[]) => number;
  "wrap-sub": (...xs: number[]) => number;
  "wrap-mul": (...xs: number[]) => number;
  "<": (...xs: number[]) => boolean;
  ">": (...xs: number[]) => boolean;
  "<=": (...xs: number[]) => boolean;
  ">=": (...xs: number[]) => boolean;
  "=": (...xs: unknown[]) => boolean;
  "!=": (...xs: unknown[]) => boolean;
  not: (x: unknown) => boolean;
  shl: (x: number, n: number) => number;
  shr: (x: number, n: number) => number;
  band: (a: number, b: number) => number;
  bor: (a: number, b: number) => number;
  bnot: (x: number) => number;
  "str->num": (s: string) => number | false;
  "number->string": (n: number) => string;
  "to-float": (x: unknown) => number;
  list: (...xs: unknown[]) => unknown[];
  length: (xs: unknown[] | string) => number;
  car: (xs: unknown[]) => unknown;
  cdr: (xs: unknown[]) => unknown[];
  cons: (x: unknown, xs: unknown[]) => unknown[];
  append: (...xss: unknown[][]) => unknown[];
  "str-cat": (...xs: unknown[]) => string;
  "str-join": (sep: string, xs: unknown[]) => string;
  "str-len": (s: string) => number;
  if: (c: boolean | unknown, t: unknown, e?: unknown) => unknown;
}
