pub mod config;
pub mod modules;
pub mod report;
pub mod runner;
pub mod sequence;
pub mod utils;

// match Java FastQC version string for byte-identical output
pub const VERSION: &str = "0.12.1";

pub const RUST_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const VERSION_BANNER: &str = concat!(
    "0.12.1\nFastQC-Rust rewrite v",
    env!("CARGO_PKG_VERSION")
);
