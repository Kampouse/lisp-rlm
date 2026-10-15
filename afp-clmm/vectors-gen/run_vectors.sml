(* Evaluate the machine-checked accrual fold on fixture vectors.
   Uses the Isabelle code-generator export (Accruef.sml) — the SAME
   equations the CLMM_Fee_Vectors pins quantify over. Native bigints
   via Code_Target_Numeral export. Standalone Poly/ML script. *)

use "Accruef.sml";

(* Peano-free helpers for the fixture table. The numeral export
   (Code_Target_Numeral) represents nat opaquely; use the converters
   exported in the module signature. *)
fun toInt n = Accruef.integer_of_nat n;

val two64 : IntInf.int = 18446744073709551616;
val two32 : IntInf.int = 4294967296;

(* cap = 1 everywhere: Lf g = 1 ; cap = g : Lf g = g.
   The fold calls the cap with a nat (the growth value), so convert
   inside — cap = g reads the nat back through the exported converter. *)
fun LfOne (_ : Accruef.nat) : Accruef.nat = Accruef.nat_of_integer 1;

fun LfG (g : Accruef.nat) : Accruef.nat =
  Accruef.nat_of_integer (Accruef.integer_of_nat g);

(* growth vectors (checkpoint starts at 0) *)
val vs : IntInf.int list = [two32, two32 + 5, two64 + 7, two64 + 9, 12345];

val rows =
  map (fn v =>
    let
      val c0 = Accruef.nat_of_integer 0;
      val gv = Accruef.nat_of_integer v;
      val owed  = toInt (Accruef.accruef LfOne c0 [gv]);
      val owed2 = toInt (Accruef.accruef LfG   c0 [gv]);
      val un    = toInt (Accruef.prodsum LfG   c0 [gv]);
    in (v, owed, owed2, un) end) vs;

(* pretty print as JSON lines *)
fun jsonRow (v, o1, o2, u) =
  "{\"g\": " ^ IntInf.toString v ^
  ", \"owed_one\": " ^ IntInf.toString o1 ^
  ", \"owed_g\": "   ^ IntInf.toString o2 ^
  ", \"undivided\": " ^ IntInf.toString u ^ "}";

val _ = print ("[" ^ String.concatWith ",\n " (map jsonRow rows) ^ "]\n");
