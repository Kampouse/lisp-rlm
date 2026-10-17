# CLMM v5 Kernel — Isabelle → OCaml Differential Oracle

Build the session:

```
cd afp-clmm/v5-kernel
isabelle build -D .
```

Export + compile the native oracle:

```
eval $(opam env)
isabelle export -d . -O . -x 'CLMM_v5_Kernel.CLMM_v5_Kernel:code/clmm_v5_kernel.ocaml'
mv CLMM_v5_Kernel.CLMM_v5_Kernel export
cp export/code/clmm_v5_kernel.ocaml clmm_v5_kernel.ml
cp clmm_v5_kernel.ml clmm_v5_kernel_full.ml
cat driver.ml >> clmm_v5_kernel_full.ml
ocamlopt -I +str -I /Users/asil/.opam/fstar/lib/zarith str.cmxa zarith.cmxa \
  clmm_v5_kernel_full.ml -o clmm_v5_quote_opt
```

Smoke test:

```
echo '{"op":"y","s":"18446744073709551616","p":"0","imin":"-2","imax":"2","v":"1000000000000000000000000000","shift":"8","lq_0":"100000000000000000000000000000"}' | \
  ./clmm_v5_quote_opt
# => {"out":271043,"s":18444899583751176498,"p":-2}
```

The Rust fuzz harness in `tests/test_pool_v5_fuzz.rs` spawns
`clmm_v5_quote_opt` and asserts S/P bit-exact equality after each swap.

Generated artifacts are ignored; only `.thy`, `ROOT`, `driver.ml`, and
`clmm_v5_kernel.ml` are versioned.
