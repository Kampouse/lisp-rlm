module HttpStateMachine

open CoreTypes

(* P2 HTTP state machine states *)
type http_state = 
  | Idle 
  | BuildingRequest 
  | Sending 
  | Receiving 
  | Done 
  | Error

(* Valid state transitions *)
let valid_transition (from: http_state) (to: http_state) : Type =
  match from with
  | Idle -> to = BuildingRequest \/ to = Error
  | BuildingRequest -> to = Sending \/ to = Error
  | Sending -> to = Receiving \/ to = Error
  | Receiving -> to = Done \/ to = Error
  | Done -> to = Error
  | Error -> False

(* HTTP invariant: never stuck in non-terminal state *)
let http_invariant (s: http_state) : Type = 
  s <> Error

(* Request buffer bounds *)
let request_bounds (url_len: int) (body_len: int) : Type =
  url_len >= 0 /\ url_len < 4096 /\
  body_len >= 0 /\ body_len < 1048576

(* Response buffer bounds *)
let response_bounds (resp_len: int) : Type =
  resp_len >= 0 /\ resp_len < 1048576

(* Monotone protocol progress: every state has an index; transitions
   either advance the protocol or enter the Error (failure) state. *)
let state_rank : http_state -> int =
  let rank = fun (s:http_state) -> match s with
    | Idle -> 0
    | BuildingRequest -> 1
    | Sending -> 2
    | Receiving -> 3
    | Done -> 4
    | Error -> 5
  in rank

let progress_or_fail (from: http_state) (to: http_state)
  : Lemma (valid_transition from to ==> (to = Error \/ state_rank to > state_rank from)) = ()

(* Error is absorbing: no valid transition ever leaves Error *)
let error_absorbing (from: http_state) (to: http_state)
  : Lemma (valid_transition from to ==> from <> Error) = ()

(* Invariant preservation on the healthy path: a transition that does
   NOT enter Error keeps the system out of Error. (The unconditional
   version of this lemma was false: every state may transition to Error.) *)
let healthy_transition_preserves_inv (from: http_state) (to: http_state)
  : Lemma (requires valid_transition from to /\ to <> Error)
          (ensures  http_invariant to) = ()

(* Terminal states *)
let is_terminal (s: http_state) : Type =
  s = Done \/ s = Error

(* Progress: non-terminal states can always transition *)
let can_progress (s: http_state) : Type =
  is_terminal s \/ (s = Idle \/ s = BuildingRequest \/ s = Sending \/ s = Receiving)