//! Validated file spans (data-model.md, Span).
//!
//! Upstream semantics (`go/client/span.go`): a span's `size` may exceed what its storage
//! holds (`sizeWithZeros`): the inline body or the block capacity holds the leading bytes,
//! the rest are zeros, and `crc` is the CRC of the stored bytes zero-extended to `size`.

use bytes::Bytes;
use rustern_msgs::{
    BlockId, BlockService, BlockServiceFlags, Crc, FetchedSpan, FetchedSpanBody, Parity,
};

use crate::crc;
use crate::layout::{Geometry, PAGE};

/// Most stripes a span has (upstream `CellsCrcs`).
pub const MAX_STRIPES: u8 = 15;

const STALE: u8 = 0x1;
const NO_READ: u8 = 0x2;
const DECOMMISSIONED: u8 = 0x8;

/// Whether a block service may be read from (upstream `BlockServiceFlags.CanRead`).
pub fn can_read(flags: BlockServiceFlags) -> bool {
    flags.0 & (STALE | NO_READ | DECOMMISSIONED) == 0
}

/// A file span, validated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    /// File offset of the first byte.
    pub byte_offset: u64,
    /// Logical bytes.
    pub size: u32,
    /// CRC32C of the logical bytes.
    pub crc: Crc,
    /// Where the bytes are.
    pub body: SpanBody,
}

/// Inline data or blocks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpanBody {
    /// Leading bytes, CRC-verified; the rest up to `size` are zeros.
    Inline {
        /// The stored bytes.
        data: Bytes,
    },
    /// Data in blocks.
    Blocks(BlocksSpan),
}

/// The block part of a span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlocksSpan {
    /// Data blocks, 1..=15.
    pub data_blocks: u8,
    /// Parity blocks (mirrors when `data_blocks == 1`).
    pub parity_blocks: u8,
    /// Stripes, 1..=15.
    pub stripes: u8,
    /// Cell size, a non-zero multiple of 4096.
    pub cell_size: u32,
    /// `data_blocks + parity_blocks` blocks, data first.
    pub blocks: Vec<BlockRef>,
    /// One CRC per stripe.
    pub stripes_crc: Vec<Crc>,
}

/// One block of a span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockRef {
    /// The service holding it.
    pub service: BlockService,
    /// Its id.
    pub block_id: BlockId,
    /// CRC of the whole block.
    pub crc: Crc,
}

/// Why a fetched span is rejected (each maps to EIO).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpanError {
    /// `parity.data_blocks` is 0.
    #[error("span has no data blocks")]
    NoDataBlocks,
    /// `blocks.len() != data + parity`.
    #[error("span has {got} blocks, parity says {want}")]
    BlockCount {
        /// From the parity.
        want: usize,
        /// In the response.
        got: usize,
    },
    /// `stripes` not in 1..=15.
    #[error("span has {0} stripes")]
    Stripes(u8),
    /// Cell size zero or not a page multiple.
    #[error("span cell size {0} is not a non-zero multiple of 4096")]
    CellSize(u32),
    /// `stripes_crc.len() != stripes`.
    #[error("span has {got} stripe CRCs for {want} stripes")]
    StripesCrc {
        /// `stripes`.
        want: usize,
        /// In the response.
        got: usize,
    },
    /// A block's size (`cell × stripes`) does not fit the 32-bit fetch request.
    #[error("span block size {0} exceeds 4 GiB")]
    BlockTooLarge(u64),
    /// `block_service_ix` out of range.
    #[error("block service index {ix} out of {len}")]
    ServiceIndex {
        /// Index.
        ix: u8,
        /// Services in the response.
        len: usize,
    },
    /// Inline body longer than the span.
    #[error("inline body of {len} bytes in a span of {size}")]
    InlineLength {
        /// Body length.
        len: usize,
        /// Span size.
        size: u32,
    },
    /// Inline body does not match the header CRC.
    #[error("inline span CRC {got:#010x}, header says {want:#010x}")]
    InlineCrc {
        /// Header.
        want: u32,
        /// Computed.
        got: u32,
    },
    /// `byte_offset + size` overflows.
    #[error("span at {byte_offset} of {size} bytes overflows")]
    Overflow {
        /// Offset.
        byte_offset: u64,
        /// Size.
        size: u32,
    },
}

impl TryFrom<(FetchedSpan, &[BlockService])> for Span {
    type Error = SpanError;

    fn try_from((f, services): (FetchedSpan, &[BlockService])) -> Result<Self, SpanError> {
        let h = f.header;
        if h.byte_offset.checked_add(u64::from(h.size)).is_none() {
            return Err(SpanError::Overflow {
                byte_offset: h.byte_offset,
                size: h.size,
            });
        }
        let body = match f.body {
            FetchedSpanBody::Inline(i) => {
                let len = i.body.len();
                let pad = usize::try_from(h.size)
                    .ok()
                    .and_then(|s| s.checked_sub(len))
                    .ok_or(SpanError::InlineLength { len, size: h.size })?;
                let got = crc::zero_extend(crc::crc32c(&i.body), pad);
                if got != h.crc.0 {
                    return Err(SpanError::InlineCrc { want: h.crc.0, got });
                }
                SpanBody::Inline { data: i.body }
            }
            FetchedSpanBody::Blocks(b) => SpanBody::Blocks(blocks(b, services)?),
        };
        Ok(Self {
            byte_offset: h.byte_offset,
            size: h.size,
            crc: h.crc,
            body,
        })
    }
}

fn blocks(
    b: rustern_msgs::FetchedBlocksSpan,
    services: &[BlockService],
) -> Result<BlocksSpan, SpanError> {
    let Parity(p) = b.parity;
    let (data_blocks, parity_blocks) = (p & 0x0F, p >> 4);
    if data_blocks == 0 {
        return Err(SpanError::NoDataBlocks);
    }
    let want = usize::from(data_blocks).saturating_add(usize::from(parity_blocks));
    if b.blocks.len() != want {
        return Err(SpanError::BlockCount {
            want,
            got: b.blocks.len(),
        });
    }
    if !(1..=MAX_STRIPES).contains(&b.stripes) {
        return Err(SpanError::Stripes(b.stripes));
    }
    if b.cell_size == 0 || u64::from(b.cell_size) % PAGE != 0 {
        return Err(SpanError::CellSize(b.cell_size));
    }
    if b.stripes_crc.len() != usize::from(b.stripes) {
        return Err(SpanError::StripesCrc {
            want: usize::from(b.stripes),
            got: b.stripes_crc.len(),
        });
    }
    let block_size = u64::from(b.cell_size).saturating_mul(u64::from(b.stripes));
    if u32::try_from(block_size).is_err() {
        return Err(SpanError::BlockTooLarge(block_size));
    }
    let blocks = b
        .blocks
        .into_iter()
        .map(|blk| {
            let service = services
                .get(usize::from(blk.block_service_ix))
                .ok_or(SpanError::ServiceIndex {
                    ix: blk.block_service_ix,
                    len: services.len(),
                })?
                .clone();
            Ok(BlockRef {
                service,
                block_id: blk.block_id,
                crc: blk.crc,
            })
        })
        .collect::<Result<_, SpanError>>()?;
    Ok(BlocksSpan {
        data_blocks,
        parity_blocks,
        stripes: b.stripes,
        cell_size: b.cell_size,
        blocks,
        stripes_crc: b.stripes_crc,
    })
}

impl Span {
    /// File offset one past the last byte (validated not to overflow).
    pub fn end(&self) -> u64 {
        self.byte_offset.saturating_add(u64::from(self.size))
    }
}

impl BlocksSpan {
    /// The layout.
    pub fn geometry(&self) -> Geometry {
        Geometry {
            data_blocks: u64::from(self.data_blocks),
            cell_size: u64::from(self.cell_size),
            stripes: u64::from(self.stripes),
        }
    }

    /// Mirrored: one data block, the others copies of it.
    pub fn is_mirrored(&self) -> bool {
        self.data_blocks == 1
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code
mod tests {
    use rustern_msgs::{
        FetchedBlock, FetchedBlocksSpan, FetchedInlineSpan, FetchedSpanHeader, StorageClass,
    };

    use super::*;

    fn services(n: u64) -> Vec<BlockService> {
        (0..n)
            .map(|i| BlockService {
                id: rustern_msgs::BlockServiceId(100 + i),
                ..BlockService::default()
            })
            .collect()
    }

    fn blocks_span(parity: u8, stripes: u8, cell_size: u32, n: usize) -> FetchedSpan {
        FetchedSpan {
            header: FetchedSpanHeader {
                byte_offset: 0,
                size: 4096,
                crc: Crc(0),
                storage_class: StorageClass(2),
            },
            body: FetchedSpanBody::Blocks(FetchedBlocksSpan {
                parity: Parity(parity),
                stripes,
                cell_size,
                blocks: (0..n)
                    .map(|i| FetchedBlock {
                        block_service_ix: u8::try_from(i).unwrap(),
                        block_id: BlockId(i as u64),
                        crc: Crc(0),
                    })
                    .collect(),
                stripes_crc: vec![Crc(0); usize::from(stripes)],
            }),
        }
    }

    fn inline(size: u32, body: &[u8], crc: u32) -> FetchedSpan {
        FetchedSpan {
            header: FetchedSpanHeader {
                byte_offset: 10,
                size,
                crc: Crc(crc),
                storage_class: rustern_msgs::INLINE_STORAGE,
            },
            body: FetchedSpanBody::Inline(FetchedInlineSpan {
                body: Bytes::copy_from_slice(body),
            }),
        }
    }

    fn check(f: FetchedSpan) -> Result<Span, SpanError> {
        Span::try_from((f, services(16).as_slice()))
    }

    #[test]
    fn valid_rs_and_mirrored() {
        let s = check(blocks_span(0x4A, 2, 8192, 14)).unwrap(); // RS(10, 4)
        let SpanBody::Blocks(b) = &s.body else {
            panic!()
        };
        assert_eq!((b.data_blocks, b.parity_blocks), (10, 4));
        assert_eq!(b.blocks[3].service.id.0, 103);
        assert!(!b.is_mirrored());
        let s = check(blocks_span(0x41, 1, 4096, 5)).unwrap(); // mirrored 1+4
        let SpanBody::Blocks(b) = &s.body else {
            panic!()
        };
        assert!(b.is_mirrored());
    }

    #[test]
    fn block_rules() {
        assert_eq!(
            check(blocks_span(0x40, 1, 4096, 4)),
            Err(SpanError::NoDataBlocks)
        );
        assert_eq!(
            check(blocks_span(0x4A, 1, 4096, 13)),
            Err(SpanError::BlockCount { want: 14, got: 13 })
        );
        assert_eq!(
            check(blocks_span(0x41, 0, 4096, 5)),
            Err(SpanError::Stripes(0))
        );
        assert_eq!(
            check(blocks_span(0x41, 16, 4096, 5)),
            Err(SpanError::Stripes(16))
        );
        assert_eq!(
            check(blocks_span(0x41, 1, 0, 5)),
            Err(SpanError::CellSize(0))
        );
        assert_eq!(
            check(blocks_span(0x41, 1, 4097, 5)),
            Err(SpanError::CellSize(4097))
        );
        let big = 4096 * 1_048_575;
        assert_eq!(
            check(blocks_span(0x41, 2, big, 5)),
            Err(SpanError::BlockTooLarge(2 * u64::from(big)))
        );
        let mut f = blocks_span(0x41, 2, 4096, 5);
        let FetchedSpanBody::Blocks(b) = &mut f.body else {
            panic!()
        };
        b.stripes_crc.pop();
        assert_eq!(check(f), Err(SpanError::StripesCrc { want: 2, got: 1 }));
        let f = blocks_span(0x41, 1, 4096, 5);
        assert_eq!(
            Span::try_from((f, services(4).as_slice())),
            Err(SpanError::ServiceIndex { ix: 4, len: 4 })
        );
    }

    #[test]
    fn size_past_capacity_is_zeros_not_an_error() {
        // Upstream `sizeWithZeros`.
        let mut f = blocks_span(0x41, 1, 4096, 5);
        f.header.size = 1 << 20;
        check(f).unwrap();
    }

    #[test]
    fn offset_overflow() {
        let mut f = blocks_span(0x41, 1, 4096, 5);
        f.header.byte_offset = u64::MAX - 10;
        assert!(matches!(check(f), Err(SpanError::Overflow { .. })));
    }

    #[test]
    fn inline_rules() {
        let body = b"hello";
        let s = check(inline(5, body, crc::crc32c(body))).unwrap();
        assert_eq!(s.end(), 15);
        // Trailing zeros: CRC is zero-extended.
        check(inline(9, body, crc::zero_extend(crc::crc32c(body), 4))).unwrap();
        assert_eq!(
            check(inline(4, body, crc::crc32c(body))),
            Err(SpanError::InlineLength { len: 5, size: 4 })
        );
        assert!(matches!(
            check(inline(5, body, crc::crc32c(b"hellp"))),
            Err(SpanError::InlineCrc { .. })
        ));
    }

    #[test]
    fn read_flags() {
        assert!(can_read(BlockServiceFlags(0)));
        assert!(can_read(BlockServiceFlags(0x4))); // NO_WRITE
        for f in [0x1, 0x2, 0x8] {
            assert!(!can_read(BlockServiceFlags(f)));
        }
    }
}
