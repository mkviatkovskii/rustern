//! CRC32C helpers matching upstream `go/core/crc32c`, built on the `crc-fast` crate.

use crc_fast::CrcAlgorithm::Crc32Iscsi;

/// CRC32C of `data`.
pub fn crc32c(data: &[u8]) -> u32 {
    crc_fast::crc32_iscsi(data)
}

/// CRC of `prev`'s data followed by `data`.
pub fn append(prev: u32, data: &[u8]) -> u32 {
    combine(prev, crc32c(data), data.len())
}

/// `crc(a ‖ b)` from `crc(a)`, `crc(b)` and `len(b)` (upstream `Append`).
pub fn combine(crc_a: u32, crc_b: u32, len_b: usize) -> u32 {
    // A CRC-32 combination fits in 32 bits; `crc-fast` widens every width to u64.
    crc_fast::checksum_combine(Crc32Iscsi, u64::from(crc_a), u64::from(crc_b), len_b as u64) as u32
}

/// `crc(a ‖ 0ⁿ)` from `crc(a)` (upstream `ZeroExtend`).
pub fn zero_extend(crc_a: u32, n: usize) -> u32 {
    combine(crc_a, zeros(n), n)
}

/// CRC of `n` zero bytes, without allocating `n` bytes.
fn zeros(n: usize) -> u32 {
    static ZEROS: [u8; 4096] = [0; 4096];
    let mut crc = 0;
    let mut left = n;
    while left > 0 {
        let chunk = left.min(ZEROS.len());
        crc = append(crc, ZEROS.get(..chunk).unwrap_or_default());
        left = left.saturating_sub(chunk);
    }
    crc
}

/// `crc(a ⊕ b)` for equal-length `a` and `b` (upstream `Xor`).
pub fn xor(crc_a: u32, crc_b: u32, len: usize) -> u32 {
    // CRC is affine: crc(a ^ b) = crc(a) ^ crc(b) ^ crc(0^len).
    crc_a ^ crc_b ^ zeros(len)
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)] // test code
mod tests {
    use proptest::prelude::*;

    use super::*;

    const GOLDENS: &str = include_str!("../../../testdata/goldens/crc32c.hex");

    fn unhex(s: &str) -> Vec<u8> {
        if s == "-" {
            return Vec::new();
        }
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn h(s: &str) -> u32 {
        u32::from_str_radix(s, 16).unwrap()
    }

    #[test]
    fn upstream_goldens() {
        let mut ops = std::collections::BTreeSet::new();
        for line in GOLDENS
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let (fields, desc) = line.split_once(" # ").unwrap();
            let f: Vec<&str> = fields.split(' ').collect();
            let (got, want) = match f[0] {
                "sum" => (crc32c(&unhex(f[1])), h(f[2])),
                "append" => (combine(h(f[1]), h(f[2]), f[3].parse().unwrap()), h(f[4])),
                "zero_extend" => (zero_extend(h(f[1]), f[2].parse().unwrap()), h(f[3])),
                "xor" => (xor(h(f[1]), h(f[2]), f[3].parse().unwrap()), h(f[4])),
                other => panic!("unknown op {other}"),
            };
            assert_eq!(got, want, "{line}: {desc}");
            ops.insert(f[0]);
        }
        assert_eq!(ops.len(), 4);
    }

    proptest! {
        #[test]
        fn append_chain_equals_whole(data in proptest::collection::vec(any::<u8>(), 0..2000), cuts in proptest::collection::vec(any::<prop::sample::Index>(), 0..5)) {
            let mut points: Vec<usize> = cuts.iter().map(|c| c.index(data.len() + 1)).collect();
            points.sort_unstable();
            let mut crc = 0;
            let mut start = 0;
            for p in points.into_iter().chain([data.len()]) {
                crc = append(crc, &data[start..p]);
                start = p;
            }
            prop_assert_eq!(crc, crc32c(&data));
        }

        #[test]
        fn combine_is_concatenation(a in proptest::collection::vec(any::<u8>(), 0..500), b in proptest::collection::vec(any::<u8>(), 0..500)) {
            let ab: Vec<u8> = a.iter().chain(&b).copied().collect();
            prop_assert_eq!(combine(crc32c(&a), crc32c(&b), b.len()), crc32c(&ab));
        }

        #[test]
        fn zero_extend_appends_zeros(a in proptest::collection::vec(any::<u8>(), 0..200), n in 0usize..10_000) {
            let mut az = a.clone();
            az.resize(a.len() + n, 0);
            prop_assert_eq!(zero_extend(crc32c(&a), n), crc32c(&az));
        }
    }
}
