//! Where span bytes live in blocks (upstream `go/client/span.go`, `prepareSpanInitiateReq`).
//!
//! A span of `D` data blocks and `S` stripes of `cell`-byte cells stores span byte `o` in
//! stripe `o / (D × cell)`, data block `(o mod (D × cell)) / cell`, at block offset
//! `stripe × cell + o mod cell`. For `D = 1` (mirroring) block offset = span offset.
//! Bytes from `capacity = D × cell × S` up to the span size are zeros and live nowhere.

/// TernFS page: the unit of block reads and of page CRCs.
pub const PAGE: u64 = 4096;

/// Shape of a block span; constructed only from a validated [`crate::span::Span`] or tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    /// Data blocks, 1..=15.
    pub data_blocks: u64,
    /// Cell size, a non-zero multiple of [`PAGE`].
    pub cell_size: u64,
    /// Stripes, 1..=15.
    pub stripes: u64,
}

/// Pages `[first_page, first_page + pages)` of one data block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockRange {
    /// Data block index.
    pub block: u64,
    /// First block page.
    pub first_page: u64,
    /// Number of pages.
    pub pages: u64,
}

impl Geometry {
    fn stripe_size(&self) -> Option<u64> {
        self.data_blocks.checked_mul(self.cell_size)
    }

    /// Bytes stored in blocks: `D × cell × S`.
    pub fn capacity(&self) -> Option<u64> {
        self.stripe_size()?.checked_mul(self.stripes)
    }

    /// Bytes per block: `cell × S`.
    pub fn block_size(&self) -> Option<u64> {
        self.cell_size.checked_mul(self.stripes)
    }

    /// `(data block, block offset)` of span byte `o`; `None` at or past capacity.
    pub fn locate(&self, o: u64) -> Option<(u64, u64)> {
        if o >= self.capacity()? {
            return None;
        }
        let stripe_size = self.stripe_size()?;
        let stripe = o.checked_div(stripe_size)?;
        let in_stripe = o.checked_rem(stripe_size)?;
        let block = in_stripe.checked_div(self.cell_size)?;
        let in_cell = in_stripe.checked_rem(self.cell_size)?;
        Some((
            block,
            stripe.checked_mul(self.cell_size)?.checked_add(in_cell)?,
        ))
    }

    /// Span offset of `block_offset` in data block `block`; `None` outside the span.
    pub fn inverse(&self, block: u64, block_offset: u64) -> Option<u64> {
        if block >= self.data_blocks || block_offset >= self.block_size()? {
            return None;
        }
        let stripe = block_offset.checked_div(self.cell_size)?;
        let in_cell = block_offset.checked_rem(self.cell_size)?;
        stripe
            .checked_mul(self.stripe_size()?)?
            .checked_add(block.checked_mul(self.cell_size)?)?
            .checked_add(in_cell)
    }

    /// `(data block, block page)` of span page `p` (span offset `p × PAGE`).
    pub fn page(&self, p: u64) -> Option<(u64, u64)> {
        let (block, off) = self.locate(p.checked_mul(PAGE)?)?;
        Some((block, off.checked_div(PAGE)?))
    }

    /// Span pages covering `[from, to)`, clamped to capacity: `(first, end)` page indices.
    pub fn span_pages(&self, from: u64, to: u64) -> Option<(u64, u64)> {
        let to = to.min(self.capacity()?);
        if from >= to {
            return Some((0, 0));
        }
        Some((from.checked_div(PAGE)?, to.div_ceil(PAGE)))
    }

    /// One contiguous page range per data block covering the stored part of span bytes
    /// `[from, to)`, page-aligned. Pages of the same block appear in the order of their
    /// span pages, so assigning span pages to blocks in order fills each range in order.
    pub fn ranges(&self, from: u64, to: u64) -> Option<Vec<BlockRange>> {
        let (first, end) = self.span_pages(from, to)?;
        let mut out: Vec<BlockRange> = Vec::new();
        if first >= end {
            return Some(out);
        }
        // The first page of each block within [first, end) is within the first stripe's
        // worth of pages; the last within the last. Blocks are visited in stripe order.
        let stripe_pages = self.stripe_size()?.checked_div(PAGE)?;
        let cell_pages = self.cell_size.checked_div(PAGE)?;
        for block in 0..self.data_blocks {
            let mut lo: Option<u64> = None;
            let mut hi = 0;
            // Candidate span pages of this block in the first and last stripe touched.
            let s_first = first.checked_div(stripe_pages)?;
            let s_last = end.checked_sub(1)?.checked_div(stripe_pages)?;
            for s in [s_first, s_last] {
                let cell_start = s
                    .checked_mul(stripe_pages)?
                    .checked_add(block.checked_mul(cell_pages)?)?;
                let cell_end = cell_start.checked_add(cell_pages)?;
                let a = cell_start.max(first);
                let b = cell_end.min(end);
                if a < b {
                    let (_, pa) = self.page(a)?;
                    let (_, pb) = self.page(b.checked_sub(1)?)?;
                    lo = Some(lo.map_or(pa, |l| l.min(pa)));
                    hi = hi.max(pb.checked_add(1)?);
                }
            }
            // Middle stripes cover the whole cell, so the range is contiguous from the
            // first touched page to the last.
            if s_last > s_first.checked_add(1)? {
                let first_full = s_first.checked_add(1)?.checked_mul(cell_pages)?;
                let last_full = s_last.checked_mul(cell_pages)?;
                lo = Some(lo.map_or(first_full, |l| l.min(first_full)));
                hi = hi.max(last_full);
            }
            if let Some(lo) = lo {
                out.push(BlockRange {
                    block,
                    first_page: lo,
                    pages: hi.checked_sub(lo)?,
                });
            }
        }
        Some(out)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)] // test code
mod tests {
    use std::collections::BTreeSet;

    use proptest::prelude::*;

    use super::*;

    fn geometry() -> impl Strategy<Value = Geometry> {
        (1u64..=15, 1u64..=4, 1u64..=15).prop_map(|(d, c, s)| Geometry {
            data_blocks: d,
            cell_size: c * PAGE,
            stripes: s,
        })
    }

    #[test]
    fn rs_layout_example() {
        // RS(10, 4)-like: 10 data blocks, 2 stripes, 8 KiB cells.
        let g = Geometry {
            data_blocks: 10,
            cell_size: 8192,
            stripes: 2,
        };
        assert_eq!(g.capacity(), Some(163_840));
        assert_eq!(g.locate(0), Some((0, 0)));
        assert_eq!(g.locate(8191), Some((0, 8191)));
        assert_eq!(g.locate(8192), Some((1, 0)));
        assert_eq!(g.locate(81_920), Some((0, 8192))); // second stripe
        assert_eq!(g.locate(81_920 + 9 * 8192 + 5), Some((9, 8197)));
        assert_eq!(g.locate(163_840), None, "past capacity is zero padding");
    }

    #[test]
    fn mirrored_block_holds_whole_span() {
        let g = Geometry {
            data_blocks: 1,
            cell_size: 3 * PAGE,
            stripes: 5,
        };
        for o in [0, 1, PAGE, 3 * PAGE + 7, 15 * PAGE - 1] {
            assert_eq!(g.locate(o), Some((0, o)));
        }
        assert_eq!(
            g.ranges(5, 9 * PAGE).unwrap(),
            [BlockRange {
                block: 0,
                first_page: 0,
                pages: 9
            }]
        );
    }

    proptest! {
        #[test]
        fn locate_matches_formula(g in geometry(), o in 0u64..1_000_000) {
            let o = o % g.capacity().unwrap();
            let sd = g.data_blocks * g.cell_size;
            let want = ((o % sd) / g.cell_size, (o / sd) * g.cell_size + o % g.cell_size);
            prop_assert_eq!(g.locate(o), Some(want));
            prop_assert_eq!(g.inverse(want.0, want.1), Some(o));
        }

        #[test]
        fn ranges_cover_exactly_once(g in geometry(), a in 0u64..2_000_000, b in 0u64..2_000_000) {
            let (from, to) = (a.min(b), a.max(b));
            let cap = g.capacity().unwrap();
            let ranges = g.ranges(from, to).unwrap();
            let mut seen = BTreeSet::new();
            for r in &ranges {
                prop_assert!(r.pages > 0);
                prop_assert!(r.block < g.data_blocks);
                for p in r.first_page..r.first_page + r.pages {
                    let o = g.inverse(r.block, p * PAGE).unwrap();
                    prop_assert_eq!(o % PAGE, 0);
                    prop_assert!(seen.insert(o / PAGE), "page {} twice", o / PAGE);
                }
            }
            let to = to.min(cap);
            let want: BTreeSet<u64> = if from < to {
                (from / PAGE..to.div_ceil(PAGE)).collect()
            } else {
                BTreeSet::new()
            };
            prop_assert_eq!(seen, want);
            // One range per block, in block order.
            let blocks: Vec<u64> = ranges.iter().map(|r| r.block).collect();
            let mut sorted = blocks.clone();
            sorted.dedup();
            prop_assert_eq!(blocks, sorted);
        }

        #[test]
        fn span_pages_in_order_fill_each_block_in_order(g in geometry(), a in 0u64..2_000_000, b in 0u64..2_000_000) {
            let (from, to) = (a.min(b), a.max(b));
            let (first, end) = g.span_pages(from, to).unwrap();
            let ranges = g.ranges(from, to).unwrap();
            let mut next: Vec<Option<u64>> = vec![None; g.data_blocks as usize];
            for r in &ranges {
                next[r.block as usize] = Some(r.first_page);
            }
            for p in first..end {
                let (block, page) = g.page(p).unwrap();
                prop_assert_eq!(next[block as usize], Some(page));
                next[block as usize] = Some(page + 1);
            }
            for r in &ranges {
                prop_assert_eq!(next[r.block as usize], Some(r.first_page + r.pages));
            }
        }
    }
}
