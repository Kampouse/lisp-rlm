module TestTupleType

type p = int * int

let f () : Tot p = (1, 2)

let g () : Tot (int * int) = (1, 2)
