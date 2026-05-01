mod emit;
mod hof;
mod storage;
mod u128;
mod logging;
mod json;
mod typing;
mod host;
mod tree_shake;

pub use emit::{
    WasmEmitter,
    compile_near, compile_near_to_wat,
    compile_pure, compile_pure_to_wat,
    compile_near_from_exprs, compile_near_to_wat_from_exprs,
    resolve_modules,
};
