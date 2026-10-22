module CLMM_v5_Kernel = struct

type int = Int_of_integer of Z.t;;

type num = One | Bit0 of num | Bit1 of num;;

let one_inta : int = Int_of_integer (Z.of_int 1);;

type 'a one = {one : 'a};;
let one _A = _A.one;;

let one_int = ({one = one_inta} : int one);;

let rec integer_of_int (Int_of_integer k) = k;;

let rec times_inta
  k l = Int_of_integer (Z.mul (integer_of_int k) (integer_of_int l));;

type 'a times = {times : 'a -> 'a -> 'a};;
let times _A = _A.times;;

type 'a power = {one_power : 'a one; times_power : 'a times};;

let times_int = ({times = times_inta} : int times);;

let power_int = ({one_power = one_int; times_power = times_int} : int power);;

let rec less_eq_int k l = Z.leq (integer_of_int k) (integer_of_int l);;

type 'a ord = {less_eq : 'a -> 'a -> bool; less : 'a -> 'a -> bool};;
let less_eq _A = _A.less_eq;;
let less _A = _A.less;;

let rec less_int k l = Z.lt (integer_of_int k) (integer_of_int l);;

let ord_int = ({less_eq = less_eq_int; less = less_int} : int ord);;

type nat = Nat of Z.t;;

let one_nata : nat = Nat (Z.of_int 1);;

let one_nat = ({one = one_nata} : nat one);;

let rec integer_of_nat (Nat x) = x;;

let rec times_nata m n = Nat (Z.mul (integer_of_nat m) (integer_of_nat n));;

let times_nat = ({times = times_nata} : nat times);;

let power_nat = ({one_power = one_nat; times_power = times_nat} : nat power);;

let ord_integer = ({less_eq = Z.leq; less = Z.lt} : Z.t ord);;

let rec max _A a b = (if less_eq _A a b then b else a);;

let rec nat_of_integer k = Nat (max ord_integer Z.zero k);;

let rec comp f g = (fun x -> f (g x));;

let rec nat x = comp nat_of_integer integer_of_int x;;

let rec plus_nat m n = Nat (Z.add (integer_of_nat m) (integer_of_nat n));;

let rec suc n = plus_nat n one_nata;;

let rec apsnd f (x, y) = (x, f y);;

let rec divmod_integer
  k l = (if Z.equal k Z.zero then (Z.zero, Z.zero)
          else (if Z.lt Z.zero l
                 then (if Z.lt Z.zero k
                        then (fun k l -> if Z.equal Z.zero l then
                               (Z.zero, l) else Z.div_rem (Z.abs k) (Z.abs l))
                               k l
                        else (let (r, s) =
                                (fun k l -> if Z.equal Z.zero l then
                                  (Z.zero, l) else Z.div_rem (Z.abs k)
                                  (Z.abs l))
                                  k l
                                in
                               (if Z.equal s Z.zero then (Z.neg r, Z.zero)
                                 else (Z.sub (Z.neg r) (Z.of_int 1),
Z.sub l s))))
                 else (if Z.equal l Z.zero then (Z.zero, k)
                        else apsnd Z.neg
                               (if Z.lt k Z.zero
                                 then (fun k l -> if Z.equal Z.zero l then
(Z.zero, l) else Z.div_rem (Z.abs k) (Z.abs l))
k l
                                 else (let (r, s) =
 (fun k l -> if Z.equal Z.zero l then (Z.zero, l) else Z.div_rem (Z.abs k)
   (Z.abs l))
   k l
 in
(if Z.equal s Z.zero then (Z.neg r, Z.zero)
  else (Z.sub (Z.neg r) (Z.of_int 1), Z.sub (Z.neg l) s)))))));;

let rec fst (x1, x2) = x1;;

let rec divide_integer k l = fst (divmod_integer k l);;

let rec divide_int
  k l = Int_of_integer (divide_integer (integer_of_int k) (integer_of_int l));;

let rec divide_nat
  m n = Nat (divide_integer (integer_of_nat m) (integer_of_nat n));;

let rec less_eq_nat m n = Z.leq (integer_of_nat m) (integer_of_nat n);;

let rec minus_nat
  m n = Nat (max ord_integer Z.zero
              (Z.sub (integer_of_nat m) (integer_of_nat n)));;

let rec equal_nat m n = Z.equal (integer_of_nat m) (integer_of_nat n);;

let zero_nat : nat = Nat Z.zero;;

let rec less_nat m n = Z.lt (integer_of_nat m) (integer_of_nat n);;

let rec power _A
  a n = (if equal_nat n zero_nat then one _A.one_power
          else times _A.times_power a (power _A a (minus_nat n one_nata)));;

let rec floor_sqrt_between
  m q n =
    (if equal_nat q zero_nat then zero_nat
      else (if equal_nat q one_nata
             then (if less_eq_nat
                        (power power_nat m (nat_of_integer (Z.of_int 2))) n &&
                        less_nat n
                          (power power_nat (suc m)
                            (nat_of_integer (Z.of_int 2)))
                    then m else zero_nat)
             else (let r = divide_nat q (nat_of_integer (Z.of_int 2)) in
                   let p = plus_nat m r in
                   let s = power power_nat p (nat_of_integer (Z.of_int 2)) in
                    (if equal_nat s n then p
                      else (if less_nat s n
                             then floor_sqrt_between (plus_nat m r)
                                    (minus_nat q r) n
                             else floor_sqrt_between m r n)))));;

let rec floor_sqrt
  n = floor_sqrt_between zero_nat
        (suc (divide_nat (suc n) (nat_of_integer (Z.of_int 2)))) n;;

let zero_int : int = Int_of_integer Z.zero;;

let rec int_of_nat n = Int_of_integer (integer_of_nat n);;

let rec uminus_int k = Int_of_integer (Z.neg (integer_of_int k));;

let rec abs_int i = (if less_int i zero_int then uminus_int i else i);;

let rec g
  i = (let j = nat (abs_int i) in
       let n =
         (if less_eq_int zero_int i
           then times_inta (power power_int (Int_of_integer (Z.of_int 10001)) j)
                  (power power_int
                    (Int_of_integer (Z.of_string "18446744073709551616"))
                    (nat_of_integer (Z.of_int 2)))
           else times_inta (power power_int (Int_of_integer (Z.of_int 10000)) j)
                  (power power_int
                    (Int_of_integer (Z.of_string "18446744073709551616"))
                    (nat_of_integer (Z.of_int 2))))
         in
       let d =
         (if less_eq_int zero_int i
           then power power_int (Int_of_integer (Z.of_int 10000)) j
           else power power_int (Int_of_integer (Z.of_int 10001)) j)
         in
       let q = divide_int n d in
        (if less_eq_int q zero_int then zero_int
          else int_of_nat (floor_sqrt (nat q))));;

let rec minus_int
  k l = Int_of_integer (Z.sub (integer_of_int k) (integer_of_int l));;

let rec dx_in
  l s s2 =
    divide_int
      (times_inta (times_inta l (minus_int s2 s))
        (Int_of_integer (Z.of_string "18446744073709551616")))
      (times_inta s s2);;

let rec dy_in
  l s s2 =
    divide_int (times_inta l (minus_int s s2))
      (Int_of_integer (Z.of_string "18446744073709551616"));;

let rec isqrt
  n = (if less_eq_int n zero_int then zero_int
        else int_of_nat (floor_sqrt (nat n)));;

let rec dx_out
  l s s2 =
    divide_int
      (times_inta (times_inta l (minus_int s s2))
        (Int_of_integer (Z.of_string "18446744073709551616")))
      (times_inta s s2);;

let rec dy_out
  l s s2 =
    divide_int (times_inta l (minus_int s2 s))
      (Int_of_integer (Z.of_string "18446744073709551616"));;

let rec fee_eff
  v shift =
    minus_int v
      (divide_int v
        (power power_int (Int_of_integer (Z.of_int 2)) (nat shift)));;

let rec plus_int
  k l = Int_of_integer (Z.add (integer_of_int k) (integer_of_int l));;

let rec s_for_dy_in
  l s dy =
    (if less_eq_int l zero_int then one_inta
      else max ord_int one_inta
             (minus_int s
               (divide_int
                 (minus_int
                   (times_inta
                     (Int_of_integer (Z.of_string "18446744073709551616"))
                     (plus_int dy one_inta))
                   one_inta)
                 l)));;

let rec walk_dn
  lq s p imin fuel acc rem =
    (if equal_nat fuel zero_nat then (acc, (s, p))
      else (if less_eq_int rem zero_int || less_eq_int p imin then (acc, (s, p))
             else (let l = lq p in
                   let s2 = g (minus_int p one_inta) in
                   let dyi = dy_in l s s2 in
                   let dxo = dx_out l s s2 in
                    (if less_eq_int dyi rem
                      then walk_dn lq s2 (minus_int p one_inta) imin
                             (minus_nat fuel one_nata) (plus_int acc dxo)
                             (minus_int rem dyi)
                      else (let s2a = s_for_dy_in l s rem in
                             (plus_int acc (dx_out l s s2a), (s2a, p)))))));;

let rec s_for_dx_in
  l s dx =
    (if less_eq_int l zero_int ||
          (less_eq_int s zero_int ||
            less_eq_int
              (times_inta l
                (Int_of_integer (Z.of_string "18446744073709551616")))
              (times_inta (plus_int dx one_inta) s))
      then Int_of_integer
             (Z.of_string "340282366920938463463374607431768211456")
      else max ord_int s
             (divide_int
               (minus_int
                 (times_inta (times_inta l s)
                   (Int_of_integer (Z.of_string "18446744073709551616")))
                 one_inta)
               (minus_int
                 (times_inta l
                   (Int_of_integer (Z.of_string "18446744073709551616")))
                 (times_inta (plus_int dx one_inta) s))));;

let rec walk_up
  lq s p imax fuel acc rem =
    (if equal_nat fuel zero_nat then (acc, (s, p))
      else (if less_eq_int rem zero_int ||
                 less_eq_int (plus_int imax one_inta) p
             then (acc, (s, p))
             else (let l = lq p in
                   let s2 = g (plus_int p one_inta) in
                   let dxi = dx_in l s s2 in
                   let dyo = dy_out l s s2 in
                    (if less_eq_int dxi rem
                      then walk_up lq s2 (plus_int p one_inta) imax
                             (minus_nat fuel one_nata) (plus_int acc dyo)
                             (minus_int rem dxi)
                      else (let s2a = s_for_dx_in l s rem in
                             (plus_int acc (dy_out l s s2a), (s2a, p)))))));;

let rec quote_x_in_fee
  lq s p imin imax v shift =
    (let v_eff = fee_eff v shift in
     let fuel = nat (minus_int (plus_int imax one_inta) p) in
      walk_up lq s p imax fuel zero_int v_eff);;

let rec quote_y_in_fee
  lq s p imin imax v shift =
    (let v_eff = fee_eff v shift in
     let fuel = nat (plus_int (minus_int p imin) one_inta) in
      walk_dn lq s p imin fuel zero_int v_eff);;

let rec quote_x_in_gross
  lq s p imin imax v shift =
    (let fuel = nat (minus_int (plus_int imax one_inta) p) in
      walk_up lq s p imax fuel zero_int v);;

let rec quote_y_in_gross
  lq s p imin imax v shift =
    (let fuel = nat (plus_int (minus_int p imin) one_inta) in
      walk_dn lq s p imin fuel zero_int v);;

end;; (*struct CLMM_v5_Kernel*)
