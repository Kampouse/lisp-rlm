module TestNoOpenList
open FStar.Char
open FStar.String

let rec f (fuel:int) (cs:list char) (acc:int) : Tot (int & list char) (decreases fuel) =
  if fuel <= 0 then (acc, cs)
  else match cs with
  | _ :: rest -> f (fuel - 1) rest acc
  | [] -> (acc, cs)
