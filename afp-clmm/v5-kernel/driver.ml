(* Driver: read JSON from stdin, call CLMM_v5_Kernel quote variants,
   print JSON triple.
   op "y" / "x" use the custody/fee model (subtract v >> shift up front).
   op "yg" / "xg" use the gross parity model (no fee skim). *)

open CLMM_v5_Kernel

let rec int_of_string s = Int_of_integer (Z.of_string s)
let string_of_int z = match z with Int_of_integer n -> Z.to_string n

let rec run_quote () =
  let line = try input_line stdin with End_of_file -> "" in
  if line = "" then ()
  else
    let trim s =
      let s = String.trim s in
      let n = String.length s in
      if n >= 2 && s.[0] = '"' && s.[n-1] = '"'
      then String.sub s 1 (n-2) else s
    in
    let fields = Str.split (Str.regexp "[,{}]") line in
    let kv = List.map (fun s ->
      match Str.split (Str.regexp_string ":") s with
      | [k; v] -> (trim k, trim v)
      | _ -> ("", "")
    ) fields in
    let get k def = try List.assoc k kv with Not_found -> def in
    let op = get "op" "" in
    let s0 = int_of_string (get "s" "0") in
    let p0 = int_of_string (get "p" "0") in
    let imin = int_of_string (get "imin" "-800000") in
    let imax = int_of_string (get "imax" "800000") in
    let v = int_of_string (get "v" "0") in
    let shift = int_of_string (get "shift" "8") in
    let lq k = int_of_string (get ("lq_" ^ string_of_int k) "0") in
    let (out, (s1, p1)) =
      if op = "y" then quote_y_in_fee lq s0 p0 imin imax v shift
      else if op = "yg" then quote_y_in_gross lq s0 p0 imin imax v shift
      else if op = "x" then quote_x_in_fee lq s0 p0 imin imax v shift
      else if op = "xg" then quote_x_in_gross lq s0 p0 imin imax v shift
      else quote_x_in_fee lq s0 p0 imin imax v shift
    in
    Printf.printf "{\"out\":%s,\"s\":%s,\"p\":%s}\n"
      (string_of_int out) (string_of_int s1) (string_of_int p1);
    flush stdout;
    run_quote ()

let () = run_quote ()
