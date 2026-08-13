//! Thin process adapter for the `funzzy` and `fzz` binaries.
//!
//! All application behavior lives in the library crate
//! (`src/app.rs` and its modules); this file only hands the process
//! entry point to the library so integration tests exercise the same
//! modules the binaries run.

fn main() {
    funzzy::app::run();
}
