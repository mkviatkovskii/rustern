//! TernFS wire protocol types and codecs.
//!
//! `generated.rs` is produced by `tools/rsgen` from upstream `go/msgs`; never edit it by
//! hand (Principle II). `spans.rs` holds the two hand-written unions, as upstream does.

pub mod frame;
#[doc(hidden)]
pub mod fuzz_support;
mod generated;
pub mod spans;
pub mod wire;

pub use generated::*;
pub use spans::{FetchedFullSpan, FetchedFullSpanBody, FetchedSpan, FetchedSpanBody};
