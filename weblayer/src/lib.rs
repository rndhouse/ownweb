//! Minimal library surface for the WebLayer package.
//!
//! WebLayer is currently distributed primarily as the `weblayer` daemon and
//! CLI binary. This library target intentionally exposes only stable package
//! metadata until a supported library API is designed.

/// The version of the WebLayer package.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
