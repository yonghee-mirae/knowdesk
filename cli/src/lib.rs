//! Shared by both binaries in this crate (`main.rs`'s `knowdesk-cli` and
//! `bin/find.rs`'s `kdfind`) - a binary under `src/bin/` can't `mod` a file
//! under `src/` directly, so the shared pieces live here instead.

pub mod cli_config;
pub mod parallel_index;
pub mod support;
