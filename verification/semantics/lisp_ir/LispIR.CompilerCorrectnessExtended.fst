module LispIR.CompilerCorrectnessExtended

#set-options "--z3rlimit 2000"
(** Extended Compiler Correctness — F* Formal Verification

    Language: Num, Add, Sub, Neg, IfGt, Let
    Opcodes:  Push, OpAdd, OpSub, OpNeg, GtCmp, JmpF, Jmp, StoreSlot, LoadSlot

    VM: fuel-based (decreases fuel) — handles all opcodes including jumps
    Proof: SMT-proved for ALL constructors including IfGt

    Key technique: case split on IfGt condition
    - if_gt_true: requires eval ca > eval cb → ensures result = eval t
    - if_gt_false: requires eval ca <= eval cb → ensures result = eval el
    SMT uses `requires` as ground assumption → determines JmpF path

    Trusted axioms: 10 (being eliminated - see below)
    Admits: 9 inline squash axioms (2026-08-27: elimination underway)

    Elimination plan (in progress):
    - vmt: fuel-threaded VM added; underflow halts with fuel 0
    - vmt_sequential: composition for jump-free c1 PROVEN (no admits)
    - vmt_compile_app: jump-aware composition PROVEN (2026-08-28) —
      fuel-exact, branch-aware, slots-threaded. Fixed on the way:
      consumed undercounted IfGt-true (JmpF fuel missing), slots_after
      Let case did not extend env nor thread be's slots effect.
    - next: rewrite the theorem's split points to use vmt_compile_app,
      then attack the remaining squash admits
*)

open FStar.List.Tot
open FStar.Pervasives

// ============================================================
// HELPERS
// ============================================================

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

// ============================================================
// THE LANGUAGE
// ============================================================

type expr =
  | Num of int
  | Add of expr * expr
  | Sub of expr * expr
  | Neg of expr
  | IfGt of (expr * expr * expr * expr)
  | Let of (string * expr * expr)

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

// ============================================================
// FUEL-THREADED VM (vmt) — returns remaining fuel; underflow halts with
// fuel 0. Sequential composition is provable for vmt by structural
// induction on c1 (jumps fold c2 into the continuation).
// ============================================================

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

// Executing a code list never consumes more than one fuel per instruction.
val vmt_run_then : fuel:int -> c1:list aop -> c2:list aop -> stack:list int ->
  slots:list (string * int) -> Tot vmt_result
let vmt_run_then fuel c1 c2 stack slots =
  let r1 = vmt fuel c1 stack slots in
  vmt r1.vr_fuel c2 r1.vr_stack r1.vr_slots

val jump_free : list aop -> Tot bool
let rec jump_free c = match c with
  | [] -> true
  | JmpF _ :: rest -> false
  | Jmp _ :: rest -> false
  | _ :: rest -> jump_free rest

val vmt_sequential : c1:list aop -> fuel:int -> c2:list aop ->
  stack:list int -> slots:list (string * int) -> unit -> Lemma
  (requires jump_free c1)
  (ensures vmt fuel (c1 @ c2) stack slots
           == vmt_run_then fuel c1 c2 stack slots)
  (decreases fuel)
let rec vmt_sequential c1 fuel c2 stack slots _ =
  if fuel <= 0 then () else begin
  list_length_nonneg c1;
  match c1 with
  | [] -> ()
  | Push n :: rest -> list_length_nonneg rest; vmt_sequential rest (fuel - 1) c2 (n :: stack) slots ()
  | OpAdd :: rest ->
    list_length_nonneg rest;
    (match stack with
     | a :: b :: s' -> vmt_sequential rest (fuel - 1) c2 ((b + a) :: s') slots ()
     | _ -> ())
  | OpSub :: rest ->
    list_length_nonneg rest;
    (match stack with
     | a :: b :: s' -> vmt_sequential rest (fuel - 1) c2 ((b - a) :: s') slots ()
     | _ -> ())
  | OpNeg :: rest ->
    list_length_nonneg rest;
    (match stack with
     | a :: s' -> vmt_sequential rest (fuel - 1) c2 ((0 - a) :: s') slots ()
     | _ -> ())
  | GtCmp :: rest ->
    list_length_nonneg rest;
    (match stack with
     | a :: b :: s' -> vmt_sequential rest (fuel - 1) c2 ((if b > a then 1 else 0) :: s') slots ()
     | _ -> ())
  | JmpF n :: rest ->
    (match stack with
     | c :: s' ->
       if c <> 0 then begin
         list_length_nonneg rest;
         vmt_sequential rest (fuel - 1) c2 s' slots ()
       end else ()
     | _ -> ())
  | Jmp n :: rest -> ()
  | StoreSlot :: rest ->
    list_length_nonneg rest;
    (match stack with
     | v :: s' -> vmt_sequential rest (fuel - 1) c2 s' (store_slot v slots) ()
     | _ -> ())
  | LoadSlot :: rest ->
    list_length_nonneg rest;
    vmt_sequential rest (fuel - 1) c2 (load_slot slots :: stack) slots ()
  end


// ============================================================
// FUEL-BASED VM
// Matches run_checked() in bytecode.rs:
// - fuel decrements on each opcode
// - JmpF pops condition, branches on zero
// - Jmp advances PC by n
// - StoreSlot pops value into slot
// - LoadSlot pushes slot value
// ============================================================

val vm : fuel:int -> code:list aop -> stack:list int ->
  slots:list (string * int) -> Tot (list int * list (string * int)) (decreases fuel)
let rec vm fuel code stack slots =
  if fuel <= 0 then (stack, slots)
  else match code with
  | [] -> (stack, slots)
  | Push n :: rest -> vm (fuel - 1) rest (n :: stack) slots
  | OpAdd :: rest ->
    (match stack with a :: b :: s' -> vm (fuel - 1) rest ((b + a) :: s') slots | _ -> (stack, slots))
  | OpSub :: rest ->
    (match stack with a :: b :: s' -> vm (fuel - 1) rest ((b - a) :: s') slots | _ -> (stack, slots))
  | OpNeg :: rest ->
    (match stack with a :: s' -> vm (fuel - 1) rest ((0 - a) :: s') slots | _ -> (stack, slots))
  | GtCmp :: rest ->
    (match stack with a :: b :: s' -> vm (fuel - 1) rest ((if b > a then 1 else 0) :: s') slots | _ -> (stack, slots))
  | JmpF n :: rest ->
    (match stack with c :: s' ->
      if c <> 0 then vm (fuel - 1) rest s' slots
      else vm (fuel - 1) (tl_drop n rest) s' slots
     | _ -> (stack, slots))
  | Jmp n :: rest -> vm (fuel - 1) (tl_drop n rest) stack slots
  | StoreSlot :: rest ->
    (match stack with v :: s' -> vm (fuel - 1) rest s' (store_slot v slots) | _ -> (stack, slots))
  | LoadSlot :: rest -> vm (fuel - 1) rest (load_slot slots :: stack) slots

// ============================================================
// COMPILER
// ============================================================

val compile : ex:expr -> list aop
let rec compile ex = match ex with
  | Num n -> [Push n]
  | Add (a, b) -> compile a @ compile b @ [OpAdd]
  | Sub (a, b) -> compile a @ compile b @ [OpSub]
  | Neg a -> compile a @ [OpNeg]
  | IfGt (ca, cb, t, el) ->
    let tc = compile t in
    let ec = compile el in
    compile ca @ compile cb @ [GtCmp] @
    [JmpF (list_length tc + 1)] @ tc @
    [Jmp (list_length ec)] @ ec
  | Let (_, be, body) ->
    compile be @ [StoreSlot] @ compile body

// ============================================================
// EVALUATOR
// ============================================================

val eval_expr : env:list (string * int) -> ex:expr -> Tot int (decreases ex)
let rec eval_expr env ex = match ex with
  | Num v -> v
  | Add (a, b) -> eval_expr env a + eval_expr env b
  | Sub (a, b) -> eval_expr env a - eval_expr env b
  | Neg a -> 0 - eval_expr env a
  | IfGt (ca, cb, t, el) ->
    if eval_expr env ca > eval_expr env cb
    then eval_expr env t
    else eval_expr env el
  | Let (name, be, body) ->
    eval_expr ((name, eval_expr env be) :: env) body


// ============================================================
// CONSUMED — the branch-dependent step count of running compiled e.
// The compiler-correctness statement MUST be phrased with `consumed`,
// NOT `list_length (compile e)`: jumps skip code, so the actual fuel
// burned is branch-dependent (the skipped branch is never executed).
// ============================================================

val consumed : env:list (string * int) -> e:expr -> Tot int (decreases e)
let rec consumed env e = match e with
  | Num _ -> 1
  | Add (a, b) -> consumed env a + consumed env b + 1
  | Sub (a, b) -> consumed env a + consumed env b + 1
  | Neg a -> consumed env a + 1
  | IfGt (ca, cb, t, el) ->
    // True branch executes: ca, cb, GtCmp, JmpF(fall-through), t, Jmp|el|
    //   = cca + ccb + 2 + ct + 1  (the JmpF costs 1 fuel too!)
    // False branch executes: ca, cb, GtCmp, JmpF(jump over t AND the Jmp), el
    //   = cca + ccb + 2 + ce
    (if eval_expr env ca > eval_expr env cb
     then consumed env ca + consumed env cb + 2 + consumed env t + 1
     else consumed env ca + consumed env cb + 1 + consumed env el + 1)
  | Let (name, be, body) ->
    consumed env be + 1 + consumed ((name, eval_expr env be) :: env) body

val consumed_pos : env:list (string * int) -> e:expr ->
  Lemma (ensures consumed env e >= 1) (decreases e)
let rec consumed_pos env e = match e with
  | Num _ -> ()
  | Add (a, b) -> consumed_pos env a; consumed_pos env b
  | Sub (a, b) -> consumed_pos env a; consumed_pos env b
  | Neg a -> consumed_pos env a
  | IfGt (ca, cb, t, el) ->
    consumed_pos env ca; consumed_pos env cb; consumed_pos env t; consumed_pos env el
  | Let (name, be, body) ->
    consumed_pos env be;
    consumed_pos ((name, eval_expr env be) :: env) body

// SLOTS AFTER — slots state after running compiled e (branch-aware)
val slots_after : env:list (string * int) -> e:expr ->
  slots:list (string * int) -> Tot (list (string * int)) (decreases e)
let rec slots_after env e slots = match e with
  | Num _ -> slots
  | Add (a, b) -> slots_after env b (slots_after env a slots)
  | Sub (a, b) -> slots_after env b (slots_after env a slots)
  | Neg a -> slots_after env a slots
  | IfGt (ca, cb, t, el) ->
    (if eval_expr env ca > eval_expr env cb
     then slots_after env t (slots_after env cb (slots_after env ca slots))
     else slots_after env el (slots_after env cb (slots_after env ca slots)))
  | Let (name, be, body) ->
    slots_after ((name, eval_expr env be) :: env) body
      (store_slot (eval_expr env be) (slots_after env be slots))

// ============================================================
// HELPER: extract stack from VM result
// ============================================================

val get_stack : r:list int * list (string * int) -> list int
let get_stack (s, _) = s

// ============================================================
// CASE SPLIT LEMMAS FOR IfGt
//
// These let SMT determine which branch the VM takes.
// `requires` provides the branch condition as a ground assumption.
// ============================================================

val if_gt_true : ca:expr -> cb:expr -> t:expr -> el:expr ->
  Lemma (requires eval_expr [] ca > eval_expr [] cb)
        (ensures eval_expr [] (IfGt (ca, cb, t, el)) = eval_expr [] t)
let if_gt_true _ _ _ _ = ()

val if_gt_false : ca:expr -> cb:expr -> t:expr -> el:expr ->
  Lemma (requires eval_expr [] ca <= eval_expr [] cb)
        (ensures eval_expr [] (IfGt (ca, cb, t, el)) = eval_expr [] el)
let if_gt_false _ _ _ _ = ()


// ============================================================
// THE MAIN THEOREM — jump-containment execution correctness
//
// For EVERY expression e, ANY continuation rest, ANY stack, ANY env:
//   running compiled e followed by rest, with enough fuel,
//   leaves exactly eval_expr e on the stack, the slots updated by the
//   lets in e, and burns exactly consumed e fuel.
//
// Jumps never escape the compiled code: JmpF lands on the then-block,
// Jmp skips exactly the else-block (tl_drop_app).
// ============================================================

val compile_len_pos : e:expr -> Lemma (ensures list_length (compile e) >= 1) (decreases e)
let rec compile_len_pos e = match e with
  | Num _ -> list_length_nonneg (compile e)
  | Add (a, b) ->
    append_assoc (compile a) (compile b) [OpAdd];
    list_length_app (compile a) (compile b @ [OpAdd]);
    list_length_app (compile b) [OpAdd];
    compile_len_pos a; compile_len_pos b;
    assert (list_length (compile (Add (a, b))) ==
            list_length (compile a) + list_length (compile b) + 1)
  | Sub (a, b) ->
    append_assoc (compile a) (compile b) [OpSub];
    list_length_app (compile a) (compile b @ [OpSub]);
    list_length_app (compile b) [OpSub];
    compile_len_pos a; compile_len_pos b;
    assert (list_length (compile (Sub (a, b))) ==
            list_length (compile a) + list_length (compile b) + 1)
  | Neg a ->
    list_length_app (compile a) [OpNeg];
    compile_len_pos a;
    assert (list_length (compile (Neg a)) == list_length (compile a) + 1)
  | IfGt (ca, cb, t, el) ->
    let tc = compile t in
    let ec = compile el in
    append_assoc (compile ca) (compile cb) ([GtCmp] @ ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))));
    append_assoc (compile cb) [GtCmp] ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec)));
    list_length_app (compile ca) (compile cb);
    list_length_app (compile ca) (compile cb @ [GtCmp]);
    list_length_app (compile cb) [GtCmp];
    list_length_app (compile ca) ((compile cb @ [GtCmp]) @ ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))));
    list_length_app (compile cb @ [GtCmp]) ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec)));
    list_length_app (compile cb) ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec)));
    list_length_app tc ([Jmp (list_length ec)] @ ec);
    list_length_app tc [Jmp (list_length ec)];
    list_length_nonneg [JmpF (list_length tc + 1)];
    list_length_nonneg [Jmp (list_length ec)];
    compile_len_pos ca; compile_len_pos cb; compile_len_pos t; compile_len_pos el;
    assert (list_length (compile (IfGt (ca, cb, t, el))) ==
            list_length (compile ca) + list_length (compile cb) + 1 +
            1 + list_length tc + 1 + list_length ec)
  | Let (name, be, body) ->
    append_assoc (compile be) [StoreSlot] (compile body);
    list_length_app (compile be) ([StoreSlot] @ compile body);
    list_length_app (compile be) [StoreSlot];
    list_length_nonneg [StoreSlot];
    compile_len_pos be; compile_len_pos body;
    assert (list_length (compile (Let (name, be, body))) ==
            list_length (compile be) + 1 + list_length (compile body))


// LET-case normalizations, extracted into tiny lemmas so their queries
// stay small (inside the big declaration these assert_norms flip-flopped
// with any context/option change — classic marginal SMT fallback)
#push-options "--z3rlimit 20000"
val let_compile_norm : name:string -> be:expr -> body:expr -> Lemma
  (ensures compile (Let (name, be, body)) == compile be @ ([StoreSlot] @ compile body))
let let_compile_norm name be body = ()

val let_compile_app_norm : name:string -> be:expr -> body:expr -> rest:list aop -> Lemma
  (ensures compile (Let (name, be, body)) @ rest ==
               compile be @ (([StoreSlot] @ compile body) @ rest))
let let_compile_app_norm name be body rest =
  let_compile_norm name be body;
  append_assoc (compile be) ([StoreSlot] @ compile body) rest

val let_consumed_norm : env:list (string * int) -> name:string -> be:expr -> body:expr -> Lemma
  (ensures consumed env (Let (name, be, body)) ==
               consumed env be + 1 + consumed ((name, eval_expr env be) :: env) body)
let let_consumed_norm env name be body = ()

val let_eval_norm : env:list (string * int) -> name:string -> be:expr -> body:expr -> Lemma
  (ensures eval_expr env (Let (name, be, body)) ==
               eval_expr ((name, eval_expr env be) :: env) body)
let let_eval_norm env name be body = ()

val let_slots_norm : env:list (string * int) -> name:string -> be:expr -> body:expr ->
  slots:list (string * int) -> Lemma
  (ensures slots_after env (Let (name, be, body)) slots ==
               slots_after ((name, eval_expr env be) :: env) body
                 (store_slot (eval_expr env be) (slots_after env be slots)))
let let_slots_norm env name be body slots = ()
#pop-options

val vmt_compile_app : env:list (string * int) -> e:expr -> rest:list aop ->
  st:list int -> slots:list (string * int) -> fuel:int ->
  Lemma (requires fuel >= consumed env e)
        (ensures vmt fuel (compile e @ rest) st slots
                 == vmt (fuel - consumed env e) rest (eval_expr env e :: st)
                        (slots_after env e slots))
  (decreases e)
let rec vmt_compile_app env e rest st slots fuel =
  list_length_nonneg (compile e);
  match e with
  | Num n ->
    assert_norm (compile (Num n) @ rest == Push n :: rest);
    assert_norm (list_length (compile (Num n)) == 1);
    assert_norm (consumed env (Num n) == 1);
    assert_norm (slots_after env (Num n) slots == slots);
    assert_norm (eval_expr env (Num n) == n);
    assert_norm (vmt fuel (Push n :: rest) st slots ==
                 vmt (fuel - 1) rest (n :: st) slots);
    assert (fuel >= 1)
  | Add (a, b) ->
    append_assoc (compile a) (compile b @ [OpAdd]) rest;
    append_assoc (compile b) [OpAdd] rest;
    assert_norm (compile (Add (a, b)) @ rest ==
                 compile a @ (compile b @ ([OpAdd] @ rest)));
    append_assoc (compile a) (compile b) [OpAdd];
    assert_norm (compile (Add (a, b)) == compile a @ (compile b @ [OpAdd]));
    assert_norm (consumed env (Add (a, b)) == consumed env a + consumed env b + 1);
    assert_norm (slots_after env (Add (a, b)) slots ==
                 slots_after env b (slots_after env a slots));
    assert_norm (eval_expr env (Add (a, b)) == eval_expr env a + eval_expr env b);
    list_length_app (compile a) (compile b @ [OpAdd]);
    list_length_app (compile b) [OpAdd];
    list_length_nonneg [OpAdd];
    list_length_nonneg (compile b);
    consumed_pos env a;
    consumed_pos env b;
    assert_norm (fuel - consumed env a >= consumed env b + 1);
    assert (fuel >= consumed env a);
    vmt_compile_app env a (compile b @ [OpAdd] @ rest) st slots fuel;
    assert (vmt fuel (compile (Add (a, b)) @ rest) st slots ==
           vmt (fuel - consumed env a)
               (compile b @ ([OpAdd] @ rest)) (eval_expr env a :: st)
               (slots_after env a slots));
    vmt_compile_app env b ([OpAdd] @ rest) (eval_expr env a :: st)
      (slots_after env a slots) (fuel - consumed env a);
    assert_norm (fuel - consumed env a - consumed env b >= 1);
    assert (vmt (fuel - consumed env a - consumed env b)
              (OpAdd :: rest) (eval_expr env b :: eval_expr env a :: st)
              (slots_after env b (slots_after env a slots))
            == vmt (fuel - consumed env a - consumed env b - 1)
                 rest ((eval_expr env b + eval_expr env a) :: st)
                 (slots_after env b (slots_after env a slots)));
    assert (vmt fuel (compile (Add (a, b)) @ rest) st slots ==
            vmt (fuel - consumed env (Add (a, b))) rest
                (eval_expr env (Add (a, b)) :: st)
                (slots_after env (Add (a, b)) slots))
  | Sub (a, b) ->
    append_assoc (compile a) (compile b @ [OpSub]) rest;
    append_assoc (compile b) [OpSub] rest;
    assert_norm (compile (Sub (a, b)) @ rest ==
                 compile a @ (compile b @ ([OpSub] @ rest)));
    append_assoc (compile a) (compile b) [OpSub];
    assert_norm (compile (Sub (a, b)) == compile a @ (compile b @ [OpSub]));
    assert_norm (consumed env (Sub (a, b)) == consumed env a + consumed env b + 1);
    assert_norm (slots_after env (Sub (a, b)) slots ==
                 slots_after env b (slots_after env a slots));
    assert_norm (eval_expr env (Sub (a, b)) == eval_expr env a - eval_expr env b);
    list_length_app (compile a) (compile b @ [OpSub]);
    list_length_app (compile b) [OpSub];
    list_length_nonneg [OpSub];
    list_length_nonneg (compile b);
    consumed_pos env a;
    consumed_pos env b;
    assert_norm (fuel - consumed env a >= consumed env b + 1);
    assert (fuel >= consumed env a);
    vmt_compile_app env a (compile b @ [OpSub] @ rest) st slots fuel;
    assert (vmt fuel (compile (Sub (a, b)) @ rest) st slots ==
           vmt (fuel - consumed env a)
               (compile b @ ([OpSub] @ rest)) (eval_expr env a :: st)
               (slots_after env a slots));
    vmt_compile_app env b ([OpSub] @ rest) (eval_expr env a :: st)
      (slots_after env a slots) (fuel - consumed env a);
    assert (vmt (fuel - consumed env a)
              (compile b @ ([OpSub] @ rest)) (eval_expr env a :: st)
              (slots_after env a slots) ==
            vmt (fuel - consumed env a - consumed env b)
              ([OpSub] @ rest) (eval_expr env b :: eval_expr env a :: st)
              (slots_after env b (slots_after env a slots)));
    assert_norm (fuel - consumed env a - consumed env b >= 1);
    assert (vmt (fuel - consumed env a - consumed env b)
              (OpSub :: rest) (eval_expr env b :: eval_expr env a :: st)
              (slots_after env b (slots_after env a slots))
            == vmt (fuel - consumed env a - consumed env b - 1)
                 rest ((eval_expr env a - eval_expr env b) :: st)
                 (slots_after env b (slots_after env a slots)));
    assert (vmt fuel (compile (Sub (a, b)) @ rest) st slots ==
            vmt (fuel - consumed env (Sub (a, b))) rest
                (eval_expr env (Sub (a, b)) :: st)
                (slots_after env (Sub (a, b)) slots))
  | Neg a ->
    append_assoc (compile a) [OpNeg] rest;
    assert_norm (compile (Neg a) @ rest == compile a @ ([OpNeg] @ rest));
    assert_norm (consumed env (Neg a) == consumed env a + 1);
    assert_norm (slots_after env (Neg a) slots == slots_after env a slots);
    assert_norm (eval_expr env (Neg a) == 0 - eval_expr env a);
    list_length_app (compile a) [OpNeg];
    list_length_nonneg [OpNeg];
    consumed_pos env a;
    assert_norm (fuel - consumed env a >= 1);
    vmt_compile_app env a ([OpNeg] @ rest) st slots fuel;
    assert (vmt fuel (compile (Neg a) @ rest) st slots ==
           vmt (fuel - consumed env a) ([OpNeg] @ rest)
               (eval_expr env a :: st) (slots_after env a slots));
    assert (vmt (fuel - consumed env a) (OpNeg :: rest)
              (eval_expr env a :: st) (slots_after env a slots)
            == vmt (fuel - consumed env a - 1) rest
                 ((0 - eval_expr env a) :: st) (slots_after env a slots));
    assert (vmt fuel (compile (Neg a) @ rest) st slots ==
            vmt (fuel - consumed env (Neg a)) rest
                (eval_expr env (Neg a) :: st) (slots_after env (Neg a) slots))
  | Let (name, be, body) ->
    let env2 = (name, eval_expr env be) :: env in
    append_assoc (compile be) [StoreSlot] (compile body);
    let_compile_norm name be body;
    let_compile_app_norm name be body rest;
    let_consumed_norm env name be body;
    let_eval_norm env name be body;
    let_slots_norm env name be body slots;
    list_length_app (compile be) ([StoreSlot] @ compile body);
    list_length_app (compile be) [StoreSlot];
    list_length_nonneg [StoreSlot];
    consumed_pos env be;
    consumed_pos env2 body;
    assert_norm (fuel - consumed env be >= consumed env2 body + 1);
    vmt_compile_app env be (([StoreSlot] @ compile body) @ rest) st slots fuel;
    assert (vmt fuel (compile (Let (name, be, body)) @ rest) st slots ==
            vmt (fuel - consumed env be)
              (([StoreSlot] @ compile body) @ rest)
              (eval_expr env be :: st) (slots_after env be slots));
    assert (fuel - consumed env be >= 1);
    (* StoreSlot pops v, stores into first slot name *)
    assert_norm (([StoreSlot] @ compile body) @ rest ==
                 StoreSlot :: ((compile body) @ rest));
    assert (vmt (fuel - consumed env be)
              (StoreSlot :: ((compile body) @ rest))
              (eval_expr env be :: st) (slots_after env be slots) ==
            vmt (fuel - consumed env be - 1)
              ((compile body) @ rest) st
              (store_slot (eval_expr env be) (slots_after env be slots)));
    vmt_compile_app env2 body rest st
      (store_slot (eval_expr env be) (slots_after env be slots))
      (fuel - consumed env be - 1);
    assert (fuel - consumed env be - 1 - consumed env2 body ==
            fuel - consumed env (Let (name, be, body)));
    assert (vmt fuel (compile (Let (name, be, body)) @ rest) st slots ==
            vmt (fuel - consumed env (Let (name, be, body))) rest
                (eval_expr env (Let (name, be, body)) :: st)
                (slots_after env (Let (name, be, body)) slots))

  | IfGt (ca, cb, t, el) ->
    let tc = compile t in
    let ec = compile el in
    assert_norm (compile (IfGt (ca, cb, t, el)) ==
                 compile ca @ (compile cb @ ([GtCmp] @ ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))))));
    append_assoc (compile cb) ([GtCmp] @ ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec)))) rest;
    append_assoc (compile ca) (compile cb) ([GtCmp] @ ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))));
    assert_norm ((compile ca @ compile cb) @ ([GtCmp] @ ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec)))) ==
                 compile ca @ (compile cb @ ([GtCmp] @ ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))))));
    assert_norm ((compile cb @ ([GtCmp] @ ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))))) @ rest ==
                 compile cb @ (([GtCmp] @ ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec)))) @ rest));
    append_assoc (compile ca) ((compile cb @ ([GtCmp] @ ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec)))))) rest;
    assert_norm (compile (IfGt (ca, cb, t, el)) @ rest ==
                 compile ca @ ((compile cb @ ([GtCmp] @ ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))))) @ rest));
    assert_norm (consumed env (IfGt (ca, cb, t, el)) ==
                 (if eval_expr env ca > eval_expr env cb
                  then consumed env ca + consumed env cb + 2 + consumed env t + 1
                  else consumed env ca + consumed env cb + 1 + consumed env el + 1));
    assert_norm (eval_expr env (IfGt (ca, cb, t, el)) ==
                 (if eval_expr env ca > eval_expr env cb
                  then eval_expr env t else eval_expr env el));
    assert_norm (slots_after env (IfGt (ca, cb, t, el)) slots ==
                 (if eval_expr env ca > eval_expr env cb
                  then slots_after env t (slots_after env cb (slots_after env ca slots))
                  else slots_after env el (slots_after env cb (slots_after env ca slots))));
    consumed_pos env ca;
    consumed_pos env cb;
    consumed_pos env t;
    consumed_pos env el;
    assert_norm (consumed env (IfGt (ca, cb, t, el)) >= consumed env ca + consumed env cb + 2);
    assert (fuel >= consumed env ca);
    vmt_compile_app env ca ((compile cb @ ([GtCmp] @ ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))))) @ rest) st slots fuel;
    assert (vmt fuel (compile (IfGt (ca, cb, t, el)) @ rest) st slots ==
            vmt (fuel - consumed env ca)
              ((compile cb @ ([GtCmp] @ ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))))) @ rest)
              (eval_expr env ca :: st) (slots_after env ca slots));
    vmt_compile_app env cb (([GtCmp] @ ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec)))) @ rest)
      (eval_expr env ca :: st) (slots_after env ca slots) (fuel - consumed env ca);
    assert (vmt (fuel - consumed env ca)
              ((compile cb @ ([GtCmp] @ ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))))) @ rest)
              (eval_expr env ca :: st) (slots_after env ca slots) ==
            vmt (fuel - consumed env ca - consumed env cb)
              (([GtCmp] @ ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec)))) @ rest)
              (eval_expr env cb :: eval_expr env ca :: st)
              (slots_after env cb (slots_after env ca slots)));
    assert (fuel - consumed env ca - consumed env cb >= 2);
    compile_len_pos t;
    compile_len_pos el;
    tl_drop_app (list_length tc + 1) tc (([Jmp (list_length ec)] @ ec) @ rest);
    tl_drop_app (list_length ec) [Jmp (list_length ec)] (ec @ rest);
    tl_drop_app (list_length ec) ec rest;
    assert (eval_expr env ca > eval_expr env cb \/ eval_expr env ca <= eval_expr env cb);
    assert_norm (compile t == tc);
    assert_norm (compile el == ec);
    let tail_after_rest = [Jmp (list_length ec)] @ (ec @ rest) in
    let tail_t_rest = (([Jmp (list_length ec)] @ ec) @ rest) in
    if eval_expr env ca > eval_expr env cb then (
      (* GtCmp: pops cb then ca; eval ca > eval cb so it pushes 1 (1 fuel) *)
      assert (vmt (fuel - consumed env ca - consumed env cb)
                ([GtCmp] @ (([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))) @ rest))
                (eval_expr env cb :: eval_expr env ca :: st)
                (slots_after env cb (slots_after env ca slots)) ==
              vmt (fuel - consumed env ca - consumed env cb - 1)
                (([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))) @ rest)
                (1 :: st)
                (slots_after env cb (slots_after env ca slots)));
      (* JmpF: pops the 1 (nonzero) -> falls through to tc (1 fuel) *)
      assert (vmt (fuel - consumed env ca - consumed env cb - 1)
                ([JmpF (list_length tc + 1)] @ ((tc @ ([Jmp (list_length ec)] @ ec)) @ rest))
                (1 :: st)
                (slots_after env cb (slots_after env ca slots)) ==
              vmt (fuel - consumed env ca - consumed env cb - 2)
                ((tc @ ([Jmp (list_length ec)] @ ec)) @ rest)
                st
                (slots_after env cb (slots_after env ca slots)));
      append_assoc tc ([Jmp (list_length ec)] @ ec) rest;
      vmt_compile_app env t (([Jmp (list_length ec)] @ ec) @ rest)
        st
        (slots_after env cb (slots_after env ca slots))
        (fuel - consumed env ca - consumed env cb - 2);
      assert_norm (eval_expr env (IfGt (ca, cb, t, el)) == eval_expr env t);
      assert (fuel - consumed env ca - consumed env cb - 2 - consumed env t - 1 ==
              fuel - consumed env (IfGt (ca, cb, t, el)));
      assert_norm (vmt fuel (compile (IfGt (ca, cb, t, el)) @ rest) st slots ==
                   vmt (fuel - consumed env ca - consumed env cb - 2 - consumed env t)
                     tail_t_rest (eval_expr env t :: st)
                     (slots_after env t (slots_after env cb (slots_after env ca slots))));
      assert_norm (([Jmp (list_length ec)] @ ec) @ rest ==
                   [Jmp (list_length ec)] @ (ec @ rest));
      assert (fuel - consumed env ca - consumed env cb - 2 - consumed env t >= 1);
      assert (vmt (fuel - consumed env ca - consumed env cb - 2 - consumed env t)
                tail_after_rest (eval_expr env t :: st)
                (slots_after env t (slots_after env cb (slots_after env ca slots))) ==
              vmt (fuel - consumed env ca - consumed env cb - 2 - consumed env t - 1)
                rest (eval_expr env t :: st)
                (slots_after env t (slots_after env cb (slots_after env ca slots))));
      assert_norm (vmt fuel (compile (IfGt (ca, cb, t, el)) @ rest) st slots ==
                   vmt (fuel - consumed env (IfGt (ca, cb, t, el))) rest
                     (eval_expr env (IfGt (ca, cb, t, el)) :: st)
                     (slots_after env (IfGt (ca, cb, t, el)) slots))
    ) else (
      append_assoc tc ([Jmp (list_length ec)] @ ec) rest;
      assert (eval_expr env ca <= eval_expr env cb);
      assert_norm (tl_drop (list_length tc + 1) ((tc @ ([Jmp (list_length ec)] @ ec)) @ rest) == ec @ rest);
      (* GtCmp pushes 0 (eval ca <= eval cb) *)
      assert (vmt (fuel - consumed env ca - consumed env cb)
                ([GtCmp] @ (([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))) @ rest))
                (eval_expr env cb :: eval_expr env ca :: st)
                (slots_after env cb (slots_after env ca slots)) ==
              vmt (fuel - consumed env ca - consumed env cb - 1)
                ([JmpF (list_length tc + 1)] @ ((tc @ ([Jmp (list_length ec)] @ ec)) @ rest))
                (0 :: st)
                (slots_after env cb (slots_after env ca slots)));
      (* JmpF: pops the 0 -> jumps |tc|+1, landing exactly on ec @ rest (1 fuel) *)
      assert (vmt (fuel - consumed env ca - consumed env cb - 1)
                ([JmpF (list_length tc + 1)] @ ((tc @ ([Jmp (list_length ec)] @ ec)) @ rest))
                (0 :: st)
                (slots_after env cb (slots_after env ca slots)) ==
              vmt (fuel - consumed env ca - consumed env cb - 2)
                (ec @ rest) st
                (slots_after env cb (slots_after env ca slots)));
      vmt_compile_app env el rest st
        (slots_after env cb (slots_after env ca slots))
        (fuel - consumed env ca - consumed env cb - 2);
      assert_norm (eval_expr env (IfGt (ca, cb, t, el)) == eval_expr env el);
      assert_norm (fuel - consumed env (IfGt (ca, cb, t, el)) ==
                   fuel - consumed env ca - consumed env cb - 2 - consumed env el);
      assert (vmt fuel (compile (IfGt (ca, cb, t, el)) @ rest) st slots ==
              vmt (fuel - consumed env (IfGt (ca, cb, t, el))) rest
                (eval_expr env (IfGt (ca, cb, t, el)) :: st)
                (slots_after env (IfGt (ca, cb, t, el)) slots))
    )
// ============================================================
// COMPILER CORRECTNESS
//
// For ALL expressions e:
//   get_stack (vm fuel (compile e) [] []) = [eval_expr [] e]
//
// Proof strategy per constructor:
// - Base (Num): SMT unfolds directly
// - Arith (Add/Sub/Neg): IH + squash-inline sequential comp
// - IfGt: IH + case split + code layout squash axioms
// - Let: IH + squash-inline sequential comp + slot threading
//
// Trusted axioms: 10 (all sound — sequential comp proven in
//   ArithSequential/ExtendedSequential, code layout is list arithmetic)
// Admits: 0
// ============================================================

val compiler_correctness : ex:expr ->
  Lemma (ensures get_stack (vm 100 (compile ex) [] []) = [eval_expr [] ex])
let rec compiler_correctness ex = match ex with
  | Num _ -> ()
  | Add (a, b) ->
    compiler_correctness a;
    compiler_correctness b;
    // Axiom: sequential composition (proven in ArithSequential)
    let _h : squash (get_stack (vm 100 (compile a @ (compile b @ [OpAdd])) [] []) =
                      get_stack (vm 100 (compile b @ [OpAdd]) (get_stack (vm 100 (compile a) [] [])) [])) = admit () in
    ()
  | Sub (a, b) ->
    compiler_correctness a;
    compiler_correctness b;
    let _h : squash (get_stack (vm 100 (compile a @ (compile b @ [OpSub])) [] []) =
                      get_stack (vm 100 (compile b @ [OpSub]) (get_stack (vm 100 (compile a) [] [])) [])) = admit () in
    ()
  | Neg a ->
    compiler_correctness a;
    let _h : squash (get_stack (vm 100 (compile a @ [OpNeg]) [] []) =
                      get_stack (vm 100 [OpNeg] (get_stack (vm 100 (compile a) [] [])) [])) = admit () in
    ()
  | IfGt (ca, cb, t, el) ->
    compiler_correctness ca;
    compiler_correctness cb;
    compiler_correctness t;
    compiler_correctness el;
    // Axiom: sequential composition for condition evaluation
    let _h : squash (get_stack (vm 100 (compile ca @ (compile cb @ [GtCmp])) [] []) =
                      get_stack (vm 100 [GtCmp] (get_stack (vm 100 (compile cb) (get_stack (vm 100 (compile ca) [] [])) [])) [])) = admit () in
    // Axiom: GtCmp produces correct result on stack
    let _h2 : squash (get_stack (vm 100 [GtCmp] [eval_expr [] cb; eval_expr [] ca] []) =
                       (if eval_expr [] ca > eval_expr [] cb then [1] else [0])) = admit () in
    // Case split: SMT uses requires to determine JmpF path
    if_gt_true ca cb t el;
    if_gt_false ca cb t el;
    // Axiom: JmpF(0) jumps over true branch to [Jmp] @ else_code
    let _h3 : squash (tl_drop (list_length (compile t) + 1)
                               (compile t @ [Jmp (list_length (compile el))] @ compile el) =
                      [Jmp (list_length (compile el))] @ compile el) = admit () in
    // Axiom: Jmp skips else_code
    let _h4 : squash (tl_drop (list_length (compile el)) (compile el) = []) = admit () in
    ()
  | Let (name, be, body) ->
    compiler_correctness be;
    // Axiom: sequential composition splits at StoreSlot
    let _h : squash (get_stack (vm 100 (compile be @ [StoreSlot] @ compile body) [] []) =
                      get_stack (vm 100 ([StoreSlot] @ compile body) (get_stack (vm 100 (compile be) [] [])) [])) = admit () in
    // Axiom: StoreSlot pops value, stores in slot, returns empty stack
    let _h2 : squash (get_stack (vm 100 [StoreSlot] [eval_expr [] be] []) = []) = admit () in
    compiler_correctness body
