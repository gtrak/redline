//! Search & references (issue 06): the embedded-ripgrep pipeline
//! (`rg`), symbol references with token-class filtering
//! (`references`), and per-buffer occur (`occur`).
//!
//! Layering (plan): this module is plain Rust — grep crates + ignore +
//! tokio (the event bus) + std threads; zero iocraft. Results stream into
//! the store through the `SearchBus` (the issue 04 bus precedent); the UI
//! renders the store's results state.

pub mod occur;
pub mod references;
pub mod rg;
