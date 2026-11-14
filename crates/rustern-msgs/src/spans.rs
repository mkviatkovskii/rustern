// Copyright 2025 XTX Markets Technologies Limited
//
// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

//! The two unions upstream encodes by hand (`go/msgs/msgs.go:1007-1075` and
//! `FetchedSpan`/`FetchedFullSpan` `Pack`/`Unpack` in `go/msgs/msgs_bincode.go`): the body
//! that follows a span header depends on the header.

use bytes::{BufMut, Bytes};

use crate::generated::{
    EMPTY_STORAGE, FetchedBlocksSpan, FetchedInlineSpan, FetchedLocations, FetchedSpanHeader,
    FetchedSpanHeaderFull, INLINE_STORAGE,
};
use crate::wire::{Decode, DecodeError, Encode, EncodeError};

const fn smaller(a: usize, b: usize) -> usize {
    if a < b { a } else { b }
}

/// Body of a [`FetchedSpan`]: inline bytes when the storage class is `INLINE`, blocks
/// otherwise.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FetchedSpanBody {
    /// Data stored in the span itself.
    Inline(FetchedInlineSpan),
    /// Data stored in blocks.
    Blocks(FetchedBlocksSpan),
}

/// Upstream `msgs.FetchedSpan`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FetchedSpan {
    /// Offset, size, CRC and storage class.
    pub header: FetchedSpanHeader,
    /// Inline data or block list, as the storage class says.
    pub body: FetchedSpanBody,
}

impl Encode for FetchedSpan {
    fn encoded_len(&self) -> usize {
        let body = match &self.body {
            FetchedSpanBody::Inline(b) => b.encoded_len(),
            FetchedSpanBody::Blocks(b) => b.encoded_len(),
        };
        self.header.encoded_len().saturating_add(body)
    }

    fn encode(&self, out: &mut impl BufMut) -> Result<(), EncodeError> {
        if self.header.storage_class == EMPTY_STORAGE {
            return Err(EncodeError::Invalid("FetchedSpan with EMPTY storage"));
        }
        let inline = self.header.storage_class == INLINE_STORAGE;
        self.header.encode(out)?;
        match &self.body {
            FetchedSpanBody::Inline(b) if inline => b.encode(out),
            FetchedSpanBody::Blocks(b) if !inline => b.encode(out),
            _ => Err(EncodeError::Invalid(
                "FetchedSpan body does not match storage class",
            )),
        }
    }
}

impl Decode for FetchedSpan {
    #[allow(clippy::arithmetic_side_effects)] // constant; overflow is a compile error
    const MIN_ENCODED_SIZE: usize = FetchedSpanHeader::MIN_ENCODED_SIZE
        + smaller(
            FetchedInlineSpan::MIN_ENCODED_SIZE,
            FetchedBlocksSpan::MIN_ENCODED_SIZE,
        );

    fn decode(buf: &mut Bytes) -> Result<Self, DecodeError> {
        let header = FetchedSpanHeader::decode(buf)?;
        let body = if header.storage_class == EMPTY_STORAGE {
            return Err(DecodeError::InvalidValue {
                what: "FetchedSpan storage class",
                value: u64::from(header.storage_class.0),
            });
        } else if header.storage_class == INLINE_STORAGE {
            FetchedSpanBody::Inline(FetchedInlineSpan::decode(buf)?)
        } else {
            FetchedSpanBody::Blocks(FetchedBlocksSpan::decode(buf)?)
        };
        Ok(Self { header, body })
    }
}

/// Body of a [`FetchedFullSpan`]: inline bytes when `header.is_inline`, block locations
/// otherwise.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FetchedFullSpanBody {
    /// Data stored in the span itself.
    Inline(FetchedInlineSpan),
    /// Blocks, per location.
    Locations(FetchedLocations),
}

/// Upstream `msgs.FetchedFullSpan`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FetchedFullSpan {
    /// Offset, size, CRC and whether the span is inline.
    pub header: FetchedSpanHeaderFull,
    /// Inline data or locations, as `header.is_inline` says.
    pub body: FetchedFullSpanBody,
}

impl Encode for FetchedFullSpan {
    fn encoded_len(&self) -> usize {
        let body = match &self.body {
            FetchedFullSpanBody::Inline(b) => b.encoded_len(),
            FetchedFullSpanBody::Locations(b) => b.encoded_len(),
        };
        self.header.encoded_len().saturating_add(body)
    }

    fn encode(&self, out: &mut impl BufMut) -> Result<(), EncodeError> {
        self.header.encode(out)?;
        match &self.body {
            FetchedFullSpanBody::Inline(b) if self.header.is_inline => b.encode(out),
            FetchedFullSpanBody::Locations(b) if !self.header.is_inline => b.encode(out),
            _ => Err(EncodeError::Invalid(
                "FetchedFullSpan body does not match is_inline",
            )),
        }
    }
}

impl Decode for FetchedFullSpan {
    #[allow(clippy::arithmetic_side_effects)] // constant; overflow is a compile error
    const MIN_ENCODED_SIZE: usize = FetchedSpanHeaderFull::MIN_ENCODED_SIZE
        + smaller(
            FetchedInlineSpan::MIN_ENCODED_SIZE,
            FetchedLocations::MIN_ENCODED_SIZE,
        );

    fn decode(buf: &mut Bytes) -> Result<Self, DecodeError> {
        let header = FetchedSpanHeaderFull::decode(buf)?;
        let body = if header.is_inline {
            FetchedFullSpanBody::Inline(FetchedInlineSpan::decode(buf)?)
        } else {
            FetchedFullSpanBody::Locations(FetchedLocations::decode(buf)?)
        };
        Ok(Self { header, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::HDD_STORAGE;
    use crate::wire::encode_to_vec;

    #[test]
    fn mismatched_bodies_fail_to_encode() {
        let span = |storage_class, body| FetchedSpan {
            header: FetchedSpanHeader {
                storage_class,
                ..Default::default()
            },
            body,
        };
        let inline = || FetchedSpanBody::Inline(FetchedInlineSpan::default());
        let blocks = || FetchedSpanBody::Blocks(FetchedBlocksSpan::default());
        assert!(encode_to_vec(&span(EMPTY_STORAGE, inline())).is_err());
        assert!(encode_to_vec(&span(INLINE_STORAGE, blocks())).is_err());
        assert!(encode_to_vec(&span(HDD_STORAGE, inline())).is_err());
        assert!(encode_to_vec(&span(HDD_STORAGE, blocks())).is_ok());

        let full = |is_inline, body| FetchedFullSpan {
            header: FetchedSpanHeaderFull {
                is_inline,
                ..Default::default()
            },
            body,
        };
        let locations = FetchedFullSpanBody::Locations(FetchedLocations::default());
        let inline = FetchedFullSpanBody::Inline(FetchedInlineSpan::default());
        assert!(encode_to_vec(&full(true, locations)).is_err());
        assert!(encode_to_vec(&full(false, inline)).is_err());
    }
}
