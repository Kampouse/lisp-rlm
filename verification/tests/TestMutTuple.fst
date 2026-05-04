module TestMutTuple

let rec f (fuel:int) : Tot (int * int) (decreases fuel) =
  if fuel <= 0 then (0, 0)
  else (1, 2)

and g (fuel:int) : Tot (int * int) (decreases fuel) =
  if fuel <= 0 then (0, 0)
  else (3, 4)
