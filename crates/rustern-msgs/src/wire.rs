//! Wire primitives of the TernFS bincode format (research R3).
//!
//! Little-endian fixed-width scalars; `string`/`[]byte` = u8 length + bytes; `Blob` and
//! lists = u16 length (u32 for fields tagged `len32`); fixed byte arrays raw; structs
//! inline. Every read is bounds-checked and returns [`DecodeError`]; nothing here panics.

use std::fmt;

use bytes::{Buf, BufMut, Bytes};

/// A message could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// The input ended before the value did.
    #[error("truncated input: {needed} more bytes needed")]
    Truncated {
        /// Bytes missing.
        needed: usize,
    },
    /// A length prefix announces more elements than the remaining input can hold.
    #[error("length {count} x {min_size} bytes exceeds the {remaining} bytes remaining")]
    LengthExceedsInput {
        /// Announced element count.
        count: usize,
        /// Smallest encoding of one element.
        min_size: usize,
        /// Bytes left in the input.
        remaining: usize,
    },
    /// Bytes were left over after a top-level message.
    #[error("{0} trailing bytes after message")]
    TrailingBytes(usize),
    /// A field holds a value its type does not allow.
    #[error("invalid value {value} for {what}")]
    InvalidValue {
        /// The field or type.
        what: &'static str,
        /// The value found.
        value: u64,
    },
}

/// A value cannot be represented on the wire.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    /// A length does not fit its length prefix.
    #[error("{what} length {len} exceeds maximum {max}")]
    TooLong {
        /// Kind of value.
        what: &'static str,
        /// Actual length.
        len: usize,
        /// Largest encodable length.
        max: usize,
    },
    /// A value is not valid for its context (for example a span body that does not match
    /// its storage class).
    #[error("invalid value for {0}")]
    Invalid(&'static str),
}

/// Types that can be written in the TernFS wire format.
pub trait Encode {
    /// Exact number of bytes [`Encode::encode`] writes.
    fn encoded_len(&self) -> usize;
    /// Appends the encoding of `self` to `out`.
    fn encode(&self, out: &mut impl BufMut) -> Result<(), EncodeError>;
}

/// Types that can be read from the TernFS wire format.
pub trait Decode: Sized {
    /// Smallest number of bytes any value of this type occupies; used to reject length
    /// prefixes that the remaining input cannot satisfy before allocating.
    const MIN_ENCODED_SIZE: usize;
    /// Reads one value from the front of `buf`.
    fn decode(buf: &mut Bytes) -> Result<Self, DecodeError>;
}

/// Encodes `value` into a new buffer.
pub fn encode_to_vec<T: Encode>(value: &T) -> Result<Vec<u8>, EncodeError> {
    let mut out = Vec::with_capacity(value.encoded_len());
    value.encode(&mut out)?;
    Ok(out)
}

/// Decodes a whole message: `input` must hold exactly one `T`.
pub fn decode_exact<T: Decode>(mut input: Bytes) -> Result<T, DecodeError> {
    let value = T::decode(&mut input)?;
    if input.has_remaining() {
        return Err(DecodeError::TrailingBytes(input.remaining()));
    }
    Ok(value)
}

fn need(buf: &Bytes, n: usize) -> Result<(), DecodeError> {
    if buf.remaining() < n {
        return Err(DecodeError::Truncated {
            needed: n.saturating_sub(buf.remaining()),
        });
    }
    Ok(())
}

/// Splits `n` bytes off the front of `buf` without copying.
pub fn take(buf: &mut Bytes, n: usize) -> Result<Bytes, DecodeError> {
    need(buf, n)?;
    Ok(buf.split_to(n))
}

macro_rules! scalar {
    ($t:ty, $put:ident, $get:ident) => {
        impl Encode for $t {
            fn encoded_len(&self) -> usize {
                size_of::<$t>()
            }
            fn encode(&self, out: &mut impl BufMut) -> Result<(), EncodeError> {
                out.$put(*self);
                Ok(())
            }
        }
        impl Decode for $t {
            const MIN_ENCODED_SIZE: usize = size_of::<$t>();
            fn decode(buf: &mut Bytes) -> Result<Self, DecodeError> {
                need(buf, size_of::<$t>())?;
                Ok(buf.$get())
            }
        }
    };
}

scalar!(u8, put_u8, get_u8);
scalar!(u16, put_u16_le, get_u16_le);
scalar!(u32, put_u32_le, get_u32_le);
scalar!(u64, put_u64_le, get_u64_le);

impl Encode for bool {
    fn encoded_len(&self) -> usize {
        1
    }
    fn encode(&self, out: &mut impl BufMut) -> Result<(), EncodeError> {
        out.put_u8(u8::from(*self));
        Ok(())
    }
}

impl Decode for bool {
    const MIN_ENCODED_SIZE: usize = 1;
    fn decode(buf: &mut Bytes) -> Result<Self, DecodeError> {
        match u8::decode(buf)? {
            0 => Ok(false),
            1 => Ok(true),
            v => Err(DecodeError::InvalidValue {
                what: "bool",
                value: u64::from(v),
            }),
        }
    }
}

impl<const N: usize> Encode for [u8; N] {
    fn encoded_len(&self) -> usize {
        N
    }
    fn encode(&self, out: &mut impl BufMut) -> Result<(), EncodeError> {
        out.put_slice(self);
        Ok(())
    }
}

impl<const N: usize> Decode for [u8; N] {
    const MIN_ENCODED_SIZE: usize = N;
    fn decode(buf: &mut Bytes) -> Result<Self, DecodeError> {
        need(buf, N)?;
        let mut a = [0u8; N];
        buf.copy_to_slice(&mut a);
        Ok(a)
    }
}

fn put_len(
    out: &mut impl BufMut,
    len: usize,
    prefix: usize,
    what: &'static str,
) -> Result<(), EncodeError> {
    let too_long = |max: usize| EncodeError::TooLong { what, len, max };
    match prefix {
        1 => out.put_u8(u8::try_from(len).map_err(|_| too_long(u8::MAX.into()))?),
        2 => out.put_u16_le(u16::try_from(len).map_err(|_| too_long(u16::MAX.into()))?),
        _ => out.put_u32_le(u32::try_from(len).map_err(|_| too_long(usize::MAX))?),
    }
    Ok(())
}

fn get_len(buf: &mut Bytes, prefix: usize) -> Result<usize, DecodeError> {
    let n = match prefix {
        1 => u64::from(u8::decode(buf)?),
        2 => u64::from(u16::decode(buf)?),
        _ => u64::from(u32::decode(buf)?),
    };
    usize::try_from(n).map_err(|_| DecodeError::InvalidValue {
        what: "length",
        value: n,
    })
}

/// `string` / `[]byte`: u8 length + bytes. Names are bytes, never `String` (they may be
/// non-UTF-8).
impl Encode for Bytes {
    fn encoded_len(&self) -> usize {
        self.len().saturating_add(1)
    }
    fn encode(&self, out: &mut impl BufMut) -> Result<(), EncodeError> {
        put_len(out, self.len(), 1, "bytes")?;
        out.put_slice(self);
        Ok(())
    }
}

impl Decode for Bytes {
    const MIN_ENCODED_SIZE: usize = 1;
    fn decode(buf: &mut Bytes) -> Result<Self, DecodeError> {
        let n = get_len(buf, 1)?;
        take(buf, n)
    }
}

macro_rules! blob {
    ($name:ident, $prefix:expr, $doc:expr) => {
        #[doc = $doc]
        #[derive(Clone, Default, PartialEq, Eq, Hash)]
        pub struct $name(pub Bytes);

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({} bytes)", stringify!($name), self.0.len())
            }
        }
        impl Encode for $name {
            fn encoded_len(&self) -> usize {
                self.0.len().saturating_add($prefix)
            }
            fn encode(&self, out: &mut impl BufMut) -> Result<(), EncodeError> {
                put_len(out, self.0.len(), $prefix, "blob")?;
                out.put_slice(&self.0);
                Ok(())
            }
        }
        impl Decode for $name {
            const MIN_ENCODED_SIZE: usize = $prefix;
            fn decode(buf: &mut Bytes) -> Result<Self, DecodeError> {
                let n = get_len(buf, $prefix)?;
                take(buf, n).map($name)
            }
        }
    };
}

blob!(Blob, 2, "Upstream `bincode.Blob`: u16 length + bytes.");
blob!(
    Blob32,
    4,
    "Upstream `bincode.Blob` tagged `len32`: u32 length + bytes."
);

fn list_len<T: Encode>(items: &[T], prefix: usize) -> usize {
    items
        .iter()
        .fold(prefix, |acc, x| acc.saturating_add(x.encoded_len()))
}

fn encode_list<T: Encode>(
    items: &[T],
    prefix: usize,
    out: &mut impl BufMut,
) -> Result<(), EncodeError> {
    put_len(out, items.len(), prefix, "list")?;
    items.iter().try_for_each(|x| x.encode(out))
}

fn decode_list<T: Decode>(buf: &mut Bytes, prefix: usize) -> Result<Vec<T>, DecodeError> {
    let count = get_len(buf, prefix)?;
    let remaining = buf.remaining();
    if count
        .checked_mul(T::MIN_ENCODED_SIZE)
        .is_none_or(|n| n > remaining)
    {
        return Err(DecodeError::LengthExceedsInput {
            count,
            min_size: T::MIN_ENCODED_SIZE,
            remaining,
        });
    }
    let mut v = Vec::with_capacity(count);
    for _ in 0..count {
        v.push(T::decode(buf)?);
    }
    Ok(v)
}

/// Lists: u16 length + elements.
impl<T: Encode> Encode for Vec<T> {
    fn encoded_len(&self) -> usize {
        list_len(self, 2)
    }
    fn encode(&self, out: &mut impl BufMut) -> Result<(), EncodeError> {
        encode_list(self, 2, out)
    }
}

impl<T: Decode> Decode for Vec<T> {
    const MIN_ENCODED_SIZE: usize = 2;
    fn decode(buf: &mut Bytes) -> Result<Self, DecodeError> {
        decode_list(buf, 2)
    }
}

/// Encoded length of a list tagged `len32`.
pub fn list32_encoded_len<T: Encode>(items: &[T]) -> usize {
    list_len(items, 4)
}

/// Encodes a list tagged `len32` (u32 length).
pub fn encode_list32<T: Encode>(items: &[T], out: &mut impl BufMut) -> Result<(), EncodeError> {
    encode_list(items, 4, out)
}

/// Decodes a list tagged `len32` (u32 length).
pub fn decode_list32<T: Decode>(buf: &mut Bytes) -> Result<Vec<T>, DecodeError> {
    decode_list(buf, 4)
}

/// A field holding a cookie, certificate, secret key or block proof (FR-017). Encoded
/// like `T`; `Debug` prints `<redacted>`.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Secret<T>(pub T);

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl<T: Encode> Encode for Secret<T> {
    fn encoded_len(&self) -> usize {
        self.0.encoded_len()
    }
    fn encode(&self, out: &mut impl BufMut) -> Result<(), EncodeError> {
        self.0.encode(out)
    }
}

impl<T: Decode> Decode for Secret<T> {
    const MIN_ENCODED_SIZE: usize = T::MIN_ENCODED_SIZE;
    fn decode(buf: &mut Bytes) -> Result<Self, DecodeError> {
        T::decode(buf).map(Secret)
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn enc<T: Encode>(v: &T) -> Vec<u8> {
        let out = encode_to_vec(v).unwrap();
        assert_eq!(out.len(), v.encoded_len());
        out
    }

    fn roundtrip<T: Encode + Decode + PartialEq + fmt::Debug>(v: T) {
        let bytes = enc(&v);
        assert_eq!(decode_exact::<T>(Bytes::from(bytes)).unwrap(), v);
    }

    /// Every strict prefix of a valid encoding is rejected as truncated, never a panic.
    fn all_prefixes_truncated<T: Decode + fmt::Debug>(full: &[u8]) {
        for n in 0..full.len() {
            let r = T::decode(&mut Bytes::copy_from_slice(&full[..n]));
            assert!(
                matches!(
                    r,
                    Err(DecodeError::Truncated { .. } | DecodeError::LengthExceedsInput { .. })
                ),
                "prefix {n}: {r:?}"
            );
        }
    }

    #[test]
    fn scalars_little_endian() {
        assert_eq!(enc(&0x12u8), [0x12]);
        assert_eq!(enc(&0x1234u16), [0x34, 0x12]);
        assert_eq!(enc(&0x1234_5678u32), [0x78, 0x56, 0x34, 0x12]);
        assert_eq!(enc(&0x0102_0304_0506_0708u64), [8, 7, 6, 5, 4, 3, 2, 1]);
        assert_eq!(enc(&true), [1]);
        roundtrip(u64::MAX);
        roundtrip(false);
    }

    #[test]
    fn bool_rejects_other_values() {
        assert!(matches!(
            bool::decode(&mut Bytes::from_static(&[2])),
            Err(DecodeError::InvalidValue { .. })
        ));
    }

    #[test]
    fn bytes_have_u8_length() {
        assert_eq!(enc(&Bytes::from_static(b"ab")), [2, b'a', b'b']);
        roundtrip(Bytes::from(vec![0xffu8; 255]));
        let err = encode_to_vec(&Bytes::from(vec![0u8; 256])).unwrap_err();
        assert!(matches!(err, EncodeError::TooLong { max: 255, .. }));
    }

    #[test]
    fn blob_and_lists_have_u16_length_len32_u32() {
        assert_eq!(enc(&Blob(Bytes::from_static(b"x"))), [1, 0, b'x']);
        assert_eq!(enc(&Blob32(Bytes::from_static(b"x"))), [1, 0, 0, 0, b'x']);
        assert_eq!(enc(&vec![1u16, 2]), [2, 0, 1, 0, 2, 0]);
        let mut out = Vec::new();
        encode_list32(&[7u8], &mut out).unwrap();
        assert_eq!(out, [1, 0, 0, 0, 7]);
        assert_eq!(list32_encoded_len(&[7u8]), 5);
        assert_eq!(decode_list32::<u8>(&mut Bytes::from(out)).unwrap(), [7]);
    }

    #[test]
    fn fixed_arrays_are_raw() {
        assert_eq!(enc(&[1u8, 2, 3, 4]), [1, 2, 3, 4]);
        roundtrip([9u8; 8]);
    }

    #[test]
    fn truncation_at_every_position() {
        all_prefixes_truncated::<u64>(&enc(&u64::MAX));
        all_prefixes_truncated::<Bytes>(&enc(&Bytes::from_static(b"hello")));
        all_prefixes_truncated::<Blob>(&enc(&Blob(Bytes::from_static(b"hello"))));
        all_prefixes_truncated::<Vec<u32>>(&enc(&vec![1u32, 2, 3]));
        all_prefixes_truncated::<[u8; 4]>(&[1, 2, 3, 4]);
    }

    #[test]
    fn huge_list_count_rejected_before_allocation() {
        // 65535 u64 elements announced, 3 bytes present.
        let r = Vec::<u64>::decode(&mut Bytes::from_static(&[0xff, 0xff, 1, 2, 3]));
        assert_eq!(
            r,
            Err(DecodeError::LengthExceedsInput {
                count: 65535,
                min_size: 8,
                remaining: 3
            })
        );
        let r = decode_list32::<u32>(&mut Bytes::from_static(&[0xff, 0xff, 0xff, 0xff]));
        assert!(matches!(r, Err(DecodeError::LengthExceedsInput { .. })));
    }

    #[test]
    fn trailing_bytes_rejected() {
        assert_eq!(
            decode_exact::<u8>(Bytes::from_static(&[1, 2])),
            Err(DecodeError::TrailingBytes(1))
        );
    }

    #[test]
    fn secret_debug_redacted() {
        let s = Secret([0xabu8; 8]);
        assert_eq!(format!("{s:?}"), "<redacted>");
        roundtrip(s);
    }

    #[test]
    fn blob_debug_shows_length_only() {
        let b = Bytes::from_static(b"abc");
        assert_eq!(format!("{:?}", Blob(b.clone())), "Blob(3 bytes)");
        assert_eq!(format!("{:?}", Blob32(b)), "Blob32(3 bytes)");
    }

    proptest! {
        #[test]
        fn arbitrary_input_never_panics(data in proptest::collection::vec(any::<u8>(), 0..64)) {
            let b = Bytes::from(data);
            let _ = decode_exact::<Vec<Bytes>>(b.clone());
            let _ = decode_exact::<Vec<Vec<u16>>>(b.clone());
            let _ = decode_exact::<Blob32>(b);
        }

        #[test]
        fn bytes_roundtrip(data in proptest::collection::vec(any::<u8>(), 0..=255)) {
            roundtrip(Bytes::from(data));
        }
    }
}
