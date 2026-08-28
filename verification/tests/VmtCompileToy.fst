module VmtCompileToy

#set-options "--z3rlimit 5000 --max_fuel 10 --max_ifuel 10"

open FStar.List.Tot

type aop =
  | Push of int
  | OpAdd
  | JmpF of int
  | Jmp of int

type expr =
  | Num of int
  | Add of expr * expr
  | IfZ of expr * expr * expr

type vmt_result = { vr_stack : list int; vr_fuel : int }

let vr_f (r:vmt_result) : int = r.vr_fuel

val tl_drop : n:int -> l:list 'a -> list 'a
let rec tl_drop n l =
  if n <= 0 then l
  else match l with [] -> [] | _ :: rest -> tl_drop (n - 1) rest

val list_length : list 'a -> int
let rec list_length l = match l with [] -> 0 | _ :: rest -> 1 + list_length rest

val list_length_nonneg : l:list 'a -> Lemma (ensures list_length l >= 0) (decreases l)
let rec list_length_nonneg l = match l with [] -> () | _ :: rest -> list_length_nonneg rest

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

val vmt : fuel:int -> code:list aop -> stack:list int -> Tot vmt_result (decreases fuel)
let rec vmt fuel code stack =
  if fuel <= 0 then { vr_stack = stack; vr_fuel = fuel }
  else match code with
  | [] -> { vr_stack = stack; vr_fuel = fuel }
  | Push n :: rest -> vmt (fuel - 1) rest (n :: stack)
  | OpAdd :: rest ->
    (match stack with
     | a :: b :: s -> vmt (fuel - 1) rest ((b + a) :: s)
     | _ -> { vr_stack = stack; vr_fuel = 0 })
  | JmpF n :: rest ->
    (match stack with
     | c :: s ->
       if c <> 0 then vmt (fuel - 1) rest s
       else vmt (fuel - 1) (tl_drop n rest) s
     | _ -> { vr_stack = stack; vr_fuel = 0 })
  | Jmp n :: rest -> vmt (fuel - 1) (tl_drop n rest) stack

val compile : e:expr -> list aop
let rec compile e = match e with
  | Num n -> [Push n]
  | Add (a, b) -> compile a @ compile b @ [OpAdd]
  | IfZ (c, t, el) ->
    compile c @ [JmpF (list_length (compile t) + 1)] @
    compile t @ [Jmp (list_length (compile el))] @ compile el

val eval_expr : e:expr -> Tot int (decreases e)
let rec eval_expr e = match e with
  | Num n -> n
  | Add (a, b) -> eval_expr a + eval_expr b
  | IfZ (c, t, el) -> if eval_expr c <> 0 then eval_expr t else eval_expr el

// Steps consumed by running compiled e to completion (branch-dependent)
val consumed : e:expr -> int
let rec consumed e = match e with
  | Num _ -> 1
  | Add (a, b) -> consumed a + consumed b + 1
  | IfZ (c, t, el) ->
    (if eval_expr c <> 0
     then consumed c + 1 + consumed t + 1
     else consumed c + 1 + consumed el)

// THE THEOREM: compiled code appends cleanly, jumps contained
val compile_len_pos : e:expr -> Lemma (ensures list_length (compile e) >= 1) (decreases e)
let rec compile_len_pos e = match e with
  | Num _ -> list_length_nonneg (compile e)
  | Add (a, b) ->
    append_assoc (compile a) (compile b) [OpAdd];
    list_length_app (compile a) (compile b @ [OpAdd]);
    list_length_app (compile b) [OpAdd];
    compile_len_pos a; compile_len_pos b;
    assert (list_length (compile (Add (a, b))) == list_length (compile a) + list_length (compile b) + 1)
  | IfZ (c, t, el) ->
    append_assoc (compile c) (compile t @ [Jmp (list_length (compile el))] @ compile el) [];
    list_length_app (compile c) ([JmpF (list_length (compile t) + 1)] @ (compile t @ [Jmp (list_length (compile el))] @ compile el));
    list_length_app (compile t) ([Jmp (list_length (compile el))] @ compile el);
    list_length_app (compile t) [Jmp (list_length (compile el))];
    list_length_nonneg [JmpF (list_length (compile t) + 1)];
    list_length_nonneg [Jmp (list_length (compile el))];
    compile_len_pos c; compile_len_pos t; compile_len_pos el;
    assert (list_length (compile (IfZ (c, t, el))) ==
            list_length (compile c) + 1 + list_length (compile t) + 1 + list_length (compile el))

val consumed_pos : e:expr -> Lemma (ensures consumed e >= 1) (decreases e)
let rec consumed_pos e = match e with
  | Num _ -> ()
  | Add (a, b) -> consumed_pos a; consumed_pos b
  | IfZ (c, t, el) -> consumed_pos c; consumed_pos t; consumed_pos el

val vmt_compile_app : e:expr -> rest:list aop -> st:list int -> fuel:int ->
  Lemma (requires fuel >= consumed e)
        (ensures vmt fuel (compile e @ rest) st
                 == vmt (fuel - consumed e) rest (eval_expr e :: st))
  (decreases e)
let rec vmt_compile_app e rest st fuel =
  list_length_nonneg (compile e);
  match e with
  | Num n ->
    assert_norm (compile (Num n) @ rest == Push n :: rest);
    assert_norm (list_length (compile (Num n)) == 1)
  | Add (a, b) ->
    append_assoc (compile a) (compile b @ [OpAdd]) rest;
    append_assoc (compile b) [OpAdd] rest;
    assert_norm (compile (Add (a, b)) @ rest ==
                 compile a @ (compile b @ ([OpAdd] @ rest)));
    append_assoc (compile a) (compile b) [OpAdd];
    assert_norm (compile (Add (a, b)) == compile a @ (compile b @ [OpAdd]));
    assert_norm (consumed (Add (a, b)) == consumed a + consumed b + 1);
    list_length_app (compile a) (compile b @ [OpAdd]);
    list_length_app (compile b) [OpAdd];
    list_length_nonneg [OpAdd];
    assert (list_length (compile (Add (a, b))) ==
            list_length (compile a) + list_length (compile b) + 1);
    list_length_nonneg (compile b);
    consumed_pos a;
    consumed_pos b;
    assert_norm (fuel - consumed a >= consumed b);
    assert (fuel >= consumed a);
    vmt_compile_app a (compile b @ [OpAdd] @ rest) st fuel;
    assert_norm (vmt fuel (compile (Add (a, b)) @ rest) st ==
           vmt (fuel - consumed a)
               (compile b @ ([OpAdd] @ rest)) (eval_expr a :: st));
    vmt_compile_app b ([OpAdd] @ rest) (eval_expr a :: st) (fuel - consumed a);
    (* after a-run: stack has eval a; b-run done; one step OpAdd *)
    assert_norm (fuel - consumed a - consumed b >= 1);
    assert (vmt (fuel - consumed a - consumed b)
              (OpAdd :: rest) (eval_expr b :: eval_expr a :: st)
            == vmt (fuel - consumed a - consumed b - 1)
                 rest ((eval_expr b + eval_expr a) :: st));
    assert_norm (eval_expr (Add (a, b)) == eval_expr a + eval_expr b);
    (* the branch goal, assembled step by step *)
    assert (vmt (fuel - consumed a - consumed b)
              ([OpAdd] @ rest)
              (eval_expr b :: eval_expr a :: st) ==
           vmt (fuel - consumed a - consumed b - 1)
               rest
               (eval_expr (Add (a, b)) :: st));
    assert (vmt fuel (compile (Add (a, b)) @ rest) st ==
            vmt (fuel - consumed (Add (a, b))) rest
                (eval_expr (Add (a, b)) :: st))
  | IfZ (c, t, el) ->
    let tc = compile t in
    let ec = compile el in
    assert_norm (compile (IfZ (c, t, el)) ==
                 compile c @ ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))));
    list_length_app (compile c) ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec)));
    list_length_app tc ([Jmp (list_length ec)] @ ec);
    list_length_app tc [Jmp (list_length ec)];
    list_length_nonneg [JmpF (list_length tc + 1)];
    list_length_nonneg [Jmp (list_length ec)];
    append_assoc (compile c)
      ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))) rest;
    assert_norm (compile (IfZ (c, t, el)) @ rest ==
                 compile c @ (([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))) @ rest));
    list_length_app (compile c) (([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))) @ rest);
    list_length_app tc ([Jmp (list_length ec)] @ ec);
    list_length_app tc [Jmp (list_length ec)];
    list_length_nonneg (([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))) @ rest);
    list_length_app (compile c) ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec)));
    list_length_nonneg ([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec)));
    consumed_pos c;
    consumed_pos t;
    consumed_pos el;
    assert_norm (consumed (IfZ (c, t, el)) >= consumed c + 1);
    assert (fuel >= consumed c);
    vmt_compile_app c (([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))) @ rest) st fuel;
    assert (vmt fuel (compile (IfZ (c, t, el)) @ rest) st ==
            vmt (fuel - consumed c)
              (([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))) @ rest)
              (eval_expr c :: st));
    assert (fuel - consumed c >= 1);
    compile_len_pos t;
    compile_len_pos el;
    tl_drop_app (list_length tc + 1) tc (([Jmp (list_length ec)] @ ec) @ rest);
    tl_drop_app (list_length ec) [Jmp (list_length ec)] (ec @ rest);
    assert (eval_expr c <> 0 \/ eval_expr c = 0);
    assert_norm (compile t == tc);
    assert_norm (compile el == ec);
    let tail_after_rest = [Jmp (list_length ec)] @ (ec @ rest) in
    let tail_t_rest = (([Jmp (list_length ec)] @ ec) @ rest) in
    if eval_expr c <> 0 then (
      assert (vmt (fuel - consumed c)
                (([JmpF (list_length tc + 1)] @ (tc @ ([Jmp (list_length ec)] @ ec))) @ rest)
                (eval_expr c :: st) ==
              vmt (fuel - consumed c - 1) ((tc @ ([Jmp (list_length ec)] @ ec)) @ rest) st);
      append_assoc (compile t) (([Jmp (list_length ec)] @ ec)) rest;
      vmt_compile_app t (([Jmp (list_length ec)] @ ec) @ rest) st (fuel - consumed c - 1);
      tl_drop_app (list_length ec) ec rest;
      assert_norm (eval_expr (IfZ (c, t, el)) == eval_expr t);
      assert (fuel - consumed c - 1 - consumed t - 1 ==
              fuel - consumed (IfZ (c, t, el)));
      assert_norm (vmt fuel (compile (IfZ (c, t, el)) @ rest) st ==
              vmt (fuel - consumed c - 1 - consumed t) tail_t_rest (eval_expr t :: st));
      assert_norm (([Jmp (list_length ec)] @ ec) @ rest ==
                   [Jmp (list_length ec)] @ (ec @ rest));
      assert (fuel - consumed c - 1 - consumed t >= 1);
      assert (vmt (fuel - consumed c - 1 - consumed t)
                tail_after_rest (eval_expr t :: st) ==
              vmt (fuel - consumed c - 1 - consumed t - 1) rest (eval_expr t :: st));
      assert_norm (vmt fuel (compile (IfZ (c, t, el)) @ rest) st ==
              vmt (fuel - consumed (IfZ (c, t, el))) rest (eval_expr (IfZ (c, t, el)) :: st))
    ) else (
      append_assoc tc ([Jmp (list_length ec)] @ ec) rest;
      assert_norm (tl_drop (list_length tc + 1) ((tc @ ([Jmp (list_length ec)] @ ec)) @ rest)
              == ec @ rest);
      assert (vmt (fuel - consumed c)
                ([JmpF (list_length tc + 1)] @ ((tc @ ([Jmp (list_length ec)] @ ec)) @ rest))
                (eval_expr c :: st) ==
              (match eval_expr c :: st with
               | c0 :: s ->
                 if c0 <> 0 then vmt (fuel - consumed c - 1)
                   (((tc @ ([Jmp (list_length ec)] @ ec)) @ rest)) s
                 else vmt (fuel - consumed c - 1)
                   (tl_drop (list_length tc + 1) ((tc @ ([Jmp (list_length ec)] @ ec)) @ rest)) s
               | _ -> { vr_stack = eval_expr c :: st; vr_fuel = 0 }));
      assert (eval_expr c = 0);
      assert_norm (tl_drop (list_length tc + 1) ((tc @ ([Jmp (list_length ec)] @ ec)) @ rest)
              == ec @ rest);
      assert (vmt (fuel - consumed c)
                ([JmpF (list_length tc + 1)] @ ((tc @ ([Jmp (list_length ec)] @ ec)) @ rest))
                (eval_expr c :: st) ==
              vmt (fuel - consumed c - 1) (ec @ rest) st);
      vmt_compile_app el rest st (fuel - consumed c - 1);
      assert_norm (eval_expr (IfZ (c, t, el)) == eval_expr el);
      assert_norm (fuel - consumed (IfZ (c, t, el)) == fuel - consumed c - 1 - consumed el);
      assert (vmt fuel (compile (IfZ (c, t, el)) @ rest) st ==
              vmt (fuel - consumed (IfZ (c, t, el))) rest (eval_expr (IfZ (c, t, el)) :: st))
    )
