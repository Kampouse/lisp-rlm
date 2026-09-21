module ProbeGt

#set-options "--z3rlimit 5000 --max_fuel 100 --max_ifuel 100"

open FStar.List.Tot

type aop =
  | Push of int
  | OpAdd
  | OpSub
  | OpNeg
  | GtCmp
  | JmpF of int
  | Jmp of int
  | StoreSlot
  | LoadSlot

val list_length : list 'a -> int
let rec list_length l = match l with [] -> 0 | _ :: rest -> 1 + list_length rest

val list_length_nonneg : l:list 'a -> Lemma (ensures list_length l >= 0)
  (decreases l)
let rec list_length_nonneg l = match l with
  | [] -> ()
  | _ :: rest -> list_length_nonneg rest

val tl_drop : n:int -> l:list 'a -> list 'a
let rec tl_drop n l =
  if n <= 0 then l
  else match l with [] -> [] | _ :: rest -> tl_drop (n - 1) rest

val store_slot : v:int -> slots:list (string * int) -> list (string * int)
let store_slot v slots =
  match slots with
  | (n, _) :: rest -> (n, v) :: rest
  | [] -> [("_", v)]

val load_slot : slots:list (string * int) -> int
let load_slot slots =
  match slots with
  | (_, v) :: _ -> v
  | [] -> 0


// ============================================================
// LIST LEMMAS (proven — used by jump containment)
// ============================================================

val list_length_app : l:list 'a -> m:list 'a ->
  Lemma (ensures list_length (l @ m) == list_length l + list_length m)
  (decreases l)
let rec list_length_app l m = match l with
  | [] -> list_length_nonneg m
  | x :: rest -> list_length_app rest m

val append_assoc : l:list 'a -> m:list 'a -> n:list 'a ->
  Lemma (ensures (l @ m) @ n == l @ (m @ n))
  (decreases l)
let rec append_assoc l m n = match l with
  | [] -> ()
  | x :: rest -> append_assoc rest m n

val list_length_zero : l:list 'a -> Lemma (requires list_length l == 0) (ensures l == [])
let list_length_zero l = match l with [] -> () | _ :: rest -> list_length_nonneg rest

val tl_drop_app : k:int -> l:list 'a -> m:list 'a ->
  Lemma (requires k >= list_length l)
        (ensures tl_drop k (l @ m) == tl_drop (k - list_length l) m)
  (decreases k)
let rec tl_drop_app k l m =
  list_length_nonneg l;
  if k <= 0 then (list_length_zero l)
  else match l with
  | [] -> assert_norm ([] @ m == m)
  | x :: rest ->
    assert_norm ((x :: rest) @ m == x :: (rest @ m));
    tl_drop_app (k - 1) rest m

type vmt_result = { vr_stack : list int; vr_slots : list (string * int); vr_fuel : int }

val vmt : fuel:int -> code:list aop -> stack:list int ->
  slots:list (string * int) -> Tot vmt_result (decreases fuel)
let rec vmt fuel code stack slots =
  if fuel <= 0 then { vr_stack = stack; vr_slots = slots; vr_fuel = fuel }
  else match code with
  | [] -> { vr_stack = stack; vr_slots = slots; vr_fuel = fuel }
  | Push n :: rest -> vmt (fuel - 1) rest (n :: stack) slots
  | OpAdd :: rest ->
    (match stack with
     | a :: b :: s' -> vmt (fuel - 1) rest ((b + a) :: s') slots
     | _ -> { vr_stack = stack; vr_slots = slots; vr_fuel = 0 })
  | OpSub :: rest ->
    (match stack with
     | a :: b :: s' -> vmt (fuel - 1) rest ((b - a) :: s') slots
     | _ -> { vr_stack = stack; vr_slots = slots; vr_fuel = 0 })
  | OpNeg :: rest ->
    (match stack with
     | a :: s' -> vmt (fuel - 1) rest ((0 - a) :: s') slots
     | _ -> { vr_stack = stack; vr_slots = slots; vr_fuel = 0 })
  | GtCmp :: rest ->
    (match stack with
     | a :: b :: s' -> vmt (fuel - 1) rest ((if b > a then 1 else 0) :: s') slots
     | _ -> { vr_stack = stack; vr_slots = slots; vr_fuel = 0 })
  | JmpF n :: rest ->
    (match stack with
     | c :: s' ->
       if c <> 0 then vmt (fuel - 1) rest s' slots
       else vmt (fuel - 1) (tl_drop n rest) s' slots
     | _ -> { vr_stack = stack; vr_slots = slots; vr_fuel = 0 })
  | Jmp n :: rest -> vmt (fuel - 1) (tl_drop n rest) stack slots
  | StoreSlot :: rest ->
    (match stack with
     | v :: s' -> vmt (fuel - 1) rest s' (store_slot v slots)
     | _ -> { vr_stack = stack; vr_slots = slots; vr_fuel = 0 })
  | LoadSlot :: rest -> vmt (fuel - 1) rest (load_slot slots :: stack) slots


val gt_step : f:int -> rest:list aop -> ecb:int -> eca:int -> st:list int ->
  slots:list (string * int) -> unit -> Lemma
  (requires f >= 1)
  (ensures vmt f (GtCmp :: rest) (ecb :: eca :: st) slots ==
           vmt (f - 1) rest
             ((if eca > ecb then 1 else 0) :: ecb :: eca :: st) slots)
let gt_conc : unit -> Lemma (ensures vmt 5 (GtCmp :: []) (3 :: 7 :: []) [] ==
      vmt 4 [] (0 :: 3 :: 7 :: []) [])
let gt_conc _ = ()

val add_step : f:int -> rest:list aop -> ecb:int -> eca:int -> st:list int ->
  slots:list (string * int) -> unit -> Lemma
  (requires f >= 1)
  (ensures vmt f (OpAdd :: rest) (ecb :: eca :: st) slots ==
           vmt (f - 1) rest ((eca + ecb) :: st) slots)
let add_step f rest ecb eca st slots _ = ()
