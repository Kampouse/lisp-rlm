//! Schnorr (BIP-340) verify for the mock NEAR crypto host.
//!
//! Delegates to the LIB copy (`lisp_rlm_wasm::builtin_schnorr`) — this file
//! used to carry a full 470-line duplicate of the field/point/hash math,
//! which drifted from the lib and violated the no-duplicated-logic rule.
//! The lib now also hosts sign/pubkey (interp surface, cross-runtime parity
//! tested in tests/test_schnorr_interp.rs).

pub use lisp_rlm_wasm::builtin_schnorr::schnorr_verify_impl;
