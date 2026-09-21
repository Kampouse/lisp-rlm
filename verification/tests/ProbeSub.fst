module ProbeSub

#set-options "--z3rlimit 5000 --max_fuel 10 --max_ifuel 10"

open FStar.List.Tot

type aop =
  | Push of int
  | OpSub

type vmt_result = { vr_stack : list int; vr_fuel : int }

val vmt : fuel:int -> code:list aop -> stack:list int -> Tot vmt_result (decreases fuel)
let rec vmt fuel code stack =
  if fuel <= 0 then { vr_stack = stack; vr_fuel = fuel }
  else match code with
  | [] -> { vr_stack = stack; vr_fuel = fuel }
  | Push n :: rest -> vmt (fuel - 1) rest (n :: stack)
  | OpSub :: rest ->
    (match stack with
     | a :: b :: s -> vmt (fuel - 1) rest ((b - a) :: s)
     | _ -> { vr_stack = stack; vr_fuel = 0 })

val step : f:int -> rest:list aop -> eb:int -> ea:int -> st:list int -> unit -> Lemma
  (requires f >= 1)
  (ensures vmt f (OpSub :: rest) (eb :: ea :: st) ==
           vmt (f - 1) rest ((eb - ea) :: st))
let step f rest eb ea st _ = ()

val byapp : f:int -> rest:list aop -> eb:int -> ea:int -> st:list int -> unit -> Lemma
  (requires f >= 1)
  (ensures vmt f ([OpSub] @ rest) (eb :: ea :: st) ==
           vmt (f - 1) rest ((eb - ea) :: st))
let byapp f rest eb ea st _ =
  assert_norm (([OpSub] @ rest) == OpSub :: rest);
  step f rest eb ea st ()
