module TestMutString
open FStar.String
open FStar.Char

let rec f (fuel:int) : Tot (int * (list char)) (decreases fuel) =
  if fuel <= 0 then (0, [])
  else (1, ['a'])

and g (fuel:int) : Tot (string * (list char)) (decreases fuel) =
  if fuel <= 0 then ("", [])
  else ("hi", ['b'])
