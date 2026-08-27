//! Extract mismatching program from the differential fuzzer.
//! Run: cargo test --test extract_mismatch -- --nocapture
//!
//! Round 4 (2026-08-27): implemented for real. Replays the EXACT draw
//! sequence of `test_differential_fuzz_medium_programs` (seed 12345,
//! num_slots = usize(4)+1, code_len = usize(15)+5) up to program #1278,
//! extracts the program + initial slots, prints them, and re-runs the
//! differential to verify whether the mismatch still reproduces on the
//! current tree.
//!
//! Shared infra (Rng, FUZZ_OPS, generator, differential_test_one) lives in
//! fuzz_common — extracted verbatim from test_differential_fuzz.rs so the
//! draw order is bit-exact.

mod fuzz_common;

use fuzz_common::*;

fn main_test() {
    let child = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024) // match the fuzz tests (deep LispVal drop)
        .spawn(|| {
            let target = 1278usize;
            let mut rng = Rng::new(12345);

            let mut found: Option<(Vec<Op>, Vec<LispVal>)> = None;
            for i in 0..=target {
                let num_slots = rng.next_usize(4) + 1;
                let code_len = rng.next_usize(15) + 5;
                let code = generate_random_program(&mut rng, num_slots, code_len);
                let mut init_slots = Vec::with_capacity(num_slots);
                for _ in 0..num_slots {
                    init_slots.push(rng.next_lisp_val());
                }
                if i == target {
                    found = Some((code, init_slots));
                }
            }

            let (code, init_slots) = found.expect("target program drawn");

            println!("=== program #1278 (seed 12345, medium loop) ===");
            println!("--- init slots ({})", init_slots.len());
            for (i, v) in init_slots.iter().enumerate() {
                println!("  slot[{}] = {:?}", i, v);
            }
            println!("--- code ({} ops)", code.len());
            for (i, op) in code.iter().enumerate() {
                println!("  {:3}: {:?}", i, op);
            }

            match differential_test_one(code, init_slots, 5000) {
                Some(desc) => {
                    println!("=== STILL MISMATCHES ===");
                    println!("{}", desc);
                    panic!("program #1278 reproduces the mismatch — see stdout");
                }
                None => {
                    println!("=== NO LONGER MISMATCHES (fixed on current tree) ===");
                }
            }
        })
        .unwrap();
    child.join().unwrap();
}

#[test]
fn extract_mismatch_1278() {
    main_test();
}
