structure Accruef : sig
  type nat
  type num
  val integer_of_nat : nat -> IntInf.int
  val lf_g : nat -> nat
  val lf_one : nat -> nat
  val deltas : nat -> nat list -> nat list
  val nat_of_integer : IntInf.int -> nat
  val accruef : (nat -> nat) -> nat -> nat list -> nat
  val prodsum : (nat -> nat) -> nat -> nat list -> nat
end = struct

datatype nat = Nat of IntInf.int;

datatype num = One | Bit0 of num | Bit1 of num;

val one_nata : nat = Nat (1 : IntInf.int);

type 'a one = {one : 'a};
val one = #one : 'a one -> 'a;

val one_nat = {one = one_nata} : nat one;

fun integer_of_nat (Nat x) = x;

fun times_nata m n = Nat (IntInf.* (integer_of_nat m, integer_of_nat n));

type 'a times = {times : 'a -> 'a -> 'a};
val times = #times : 'a times -> 'a -> 'a -> 'a;

type 'a power = {one_power : 'a one, times_power : 'a times};
val one_power = #one_power : 'a power -> 'a one;
val times_power = #times_power : 'a power -> 'a times;

val times_nat = {times = times_nata} : nat times;

val power_nat = {one_power = one_nat, times_power = times_nat} : nat power;

type 'a ord = {less_eq : 'a -> 'a -> bool, less : 'a -> 'a -> bool};
val less_eq = #less_eq : 'a ord -> 'a -> 'a -> bool;
val less = #less : 'a ord -> 'a -> 'a -> bool;

val ord_integer =
  {less_eq = (fn a => fn b => IntInf.<= (a, b)),
    less = (fn a => fn b => IntInf.< (a, b))}
  : IntInf.int ord;

fun apsnd f (x, y) = (x, f y);

fun lf_g g = g;

fun fst (x1, x2) = x1;

fun lf_one uu = one_nata;

fun max A_ a b = (if less_eq A_ a b then b else a);

fun minus_nat m n =
  Nat (max ord_integer (0 : IntInf.int)
        (IntInf.- (integer_of_nat m, integer_of_nat n)));

fun equal_nat m n = (((integer_of_nat m) : IntInf.int) = (integer_of_nat n));

val zero_nat : nat = Nat (0 : IntInf.int);

fun power A_ a n =
  (if equal_nat n zero_nat then one (one_power A_)
    else times (times_power A_) a (power A_ a (minus_nat n one_nata)));

fun deltas uu [] = []
  | deltas c (g :: gs) = minus_nat g c :: deltas g gs;

fun divmod_integer k l =
  (if ((k : IntInf.int) = (0 : IntInf.int))
    then ((0 : IntInf.int), (0 : IntInf.int))
    else (if IntInf.< ((0 : IntInf.int), l)
           then (if IntInf.< ((0 : IntInf.int), k)
                  then IntInf.divMod (IntInf.abs k, IntInf.abs l)
                  else let
                         val (r, s) =
                           IntInf.divMod (IntInf.abs k, IntInf.abs l);
                       in
                         (if ((s : IntInf.int) = (0 : IntInf.int))
                           then (IntInf.~ r, (0 : IntInf.int))
                           else (IntInf.- (IntInf.~ r, (1 : IntInf.int)),
                                  IntInf.- (l, s)))
                       end)
           else (if ((l : IntInf.int) = (0 : IntInf.int))
                  then ((0 : IntInf.int), k)
                  else apsnd IntInf.~
                         (if IntInf.< (k, (0 : IntInf.int))
                           then IntInf.divMod (IntInf.abs k, IntInf.abs l)
                           else let
                                  val (r, s) =
                                    IntInf.divMod (IntInf.abs k, IntInf.abs l);
                                in
                                  (if ((s : IntInf.int) = (0 : IntInf.int))
                                    then (IntInf.~ r, (0 : IntInf.int))
                                    else (IntInf.- (IntInf.~
              r, (1 : IntInf.int)),
   IntInf.- (IntInf.~ l, s)))
                                end))));

fun divide_integer k l = fst (divmod_integer k l);

fun divide_nat m n = Nat (divide_integer (integer_of_nat m) (integer_of_nat n));

fun nat_of_integer k = Nat (max ord_integer (0 : IntInf.int) k);

fun plus_nat m n = Nat (IntInf.+ (integer_of_nat m, integer_of_nat n));

fun accruef lf uu [] = zero_nat
  | accruef lf c (g :: gs) =
    plus_nat
      (divide_nat (times_nata (minus_nat g c) (lf g))
        (power power_nat (nat_of_integer (2 : IntInf.int))
          (nat_of_integer (64 : IntInf.int))))
      (accruef lf g gs);

fun prodsum lf uu [] = zero_nat
  | prodsum lf c (g :: gs) =
    plus_nat (times_nata (minus_nat g c) (lf g)) (prodsum lf g gs);

end; (*struct Accruef*)
