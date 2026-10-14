//! Cross-process serialization for tests that touch the repo-relative
//! `runtime/` dirs (state, patches).
//!
//! Two racing layers to defeat:
//!
//! 1. Test THREADS inside one test binary — a `Mutex<()>` static only guards
//!    threads of one process, AND it must be held through the whole test body.
//!    `test_harness_full.rs` used to acquire-and-drop inside `fresh()`, leaving
//!    28 sibling threads free to `remove_dir_all("runtime/state")` between a
//!    test's checkpoint and its assertions (observed: test_restore_loads_
//!    intentions asserting on a state dir another test had just nuked).
//! 2. Test BINARIES across cargo processes — `cargo test` runs each
//!    tests/*.rs crate as its own process with its own mutexes, and on this
//!    machine gate runs legitimately overlap (background gate + foreground
//!    probe). In-process locks do nothing there; flock(2) does.
//!
//! `runtime_lock()` therefore takes a blocking BSD flock via
//! `std::fs::File::lock_exclusive` (stable 1.89): exclusive per open file
//! description, so it serializes threads AND processes. The returned guard
//! releases on drop — hold it for the entire test body:
//!
//! ```ignore
//! let (_lock, mut env, mut state) = fresh();
//! ```
//!
//! The lock file itself is intentionally NEVER unlinked: unlinking a flock'd
//! file races a second process's open() into locking a different inode than
//! a third process creates — the classic flock-unlink hole. A persistent
//! zero-byte lockfile has no such race. (It is gitignored.)

use std::fs::{File, OpenOptions};

pub struct StateLock(File);

pub fn runtime_lock() -> StateLock {
    std::fs::create_dir_all("runtime").expect("create runtime/ dir");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open("runtime/.test.lock")
        .expect("open runtime/.test.lock");
    // Blocking EXCLUSIVE lock (stable since 1.89): serializes test threads
    // and other cargo-test processes alike; released when the File drops.
    file.lock().expect("flock runtime/.test.lock (blocking)");
    StateLock(file)
}
