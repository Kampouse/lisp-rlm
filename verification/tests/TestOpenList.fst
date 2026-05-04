module TestOpenList
open FStar.List.Tot
open FStar.Char
open FStar.String

let rec f (fuel:int) (cs:list char) (acc:int) : Tot (int * (list char)) (decreases fuel) =
  if fuel <= 0 then (acc, cs)
  else match cs with
  | _ :: rest -> f (fuel - 1) rest acc
  | [] -> (acc, cs)

and g (fuel:int) (cs:list char) (acc:list char) : Tot (string * (list char)) (decreases fuel) =
  if fuel <= 0 then ("", cs)
  else match cs with
  | _ :: rest -> g (fuel - 1) rest (c :: acc)
  | [] -> ("", [])
