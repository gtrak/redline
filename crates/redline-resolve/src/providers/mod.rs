//! Per-language tooling providers (plan 006). Each provider owns exactly
//! one module file and is registered into the chain by the app lane
//! (plan 006 issue 02) — parallel worker lanes each own one file.

pub mod go_provider;
pub mod js_provider;
pub mod python_provider;
