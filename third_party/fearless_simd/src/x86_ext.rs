// Copyright 2026 the Fearless_SIMD Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Extra x86 tokens for [`kernel!`](crate::kernel) only, named and defined after the GCC and
//! LLVM `-march` / `-C target-cpu` CPUs: [`Alderlake`] and [`SkylakeAvx512`].
//!
//! The [`Avx512`] level needs the whole Ice Lake feature set, so it is unavailable on
//! Skylake-SP, Cascade Lake and Cooper Lake, which only have AVX-512 F, BW, CD, DQ and VL.
//! Consumer Intel cores since Alder Lake have no AVX-512 but add GFNI, VAES, VPCLMULQDQ,
//! AVX-VNNI and SHA on top of AVX2. These tokens reach both. They are not [`Level`]s: they
//! have no [`Simd`](crate::Simd) implementation and only select the target features of a
//! `kernel!` body. Each converts to the weaker tokens it implies.

use crate::{Avx2, Avx512, Level};

#[cfg(feature = "std")]
use std::arch::is_x86_feature_detected;

macro_rules! token {
    ($(#[$meta:meta])* $name:ident, $cpu:literal, $features:literal, $list:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug)]
        pub struct $name {
            _private: (),
        }

        impl $name {
            #[doc = concat!("Create a SIMD token proving that the ", $cpu, " features are available.")]
            ///
            /// Most users should safely obtain the token from [`Level::new`] and the `Level::as_*`
            /// methods instead of calling this function.
            ///
            /// This function can be called without an `unsafe` block from a function with all
            /// the required target features enabled via the `#[target_feature]` annotation.
            ///
            /// # Safety
            ///
            /// When invoking this function through an `unsafe` block, the caller must ensure
            #[doc = concat!("that the current CPU supports ", $list, ".")]
            #[inline]
            #[target_feature(enable = $features)]
            pub const fn assume_supported() -> Self {
                Self { _private: () }
            }
        }
    };
}

// Each feature string is `rustc --print=cfg -C target-cpu=<cpu>`, pruned of the features
// implied by the others like the `Avx512` level's. Keep it identical to the token's arm in
// `__fearless_simd_kernel_target_fn!` and to its detection function below.
// `Avx2` plus adx, aes, avxvnni, gfni, pclmulqdq, rdrand, rdseed, sha, vaes, vpclmulqdq,
// xsavec, xsaveopt, xsaves.
token!(
    /// A token for the Alder Lake feature set (`-march=alderlake`) on `x86` and `x86_64`:
    /// x86-64-v3 plus GFNI, VAES, VPCLMULQDQ, AVX-VNNI and SHA on 128- and 256-bit vectors,
    /// as on Alder Lake, Raptor Lake, Meteor Lake, Arrow Lake, Lunar Lake and Gracemont.
    ///
    /// Key Locker (`kl`, `widekl`), which `-march=alderlake` also enables, is left out: it is
    /// not a vector extension and firmware or the OS often disables it.
    ///
    /// Obtain it with [`Level::as_alderlake`], then pass it to a function defined with
    /// [`kernel!`](crate::kernel) to call those intrinsics safely.
    Alderlake,
    "Alder Lake",
    "fxsr,adx,aes,avx2,avxvnni,bmi1,bmi2,cmpxchg16b,f16c,fma,gfni,lzcnt,movbe,pclmulqdq,popcnt,rdrand,rdseed,sha,vaes,vpclmulqdq,xsave,xsavec,xsaveopt,xsaves",
    "`adx`, `aes`, `avx2`, `avxvnni`, `bmi1`, `bmi2`, `cmpxchg16b`, `f16c`, `fma`, `fxsr`, `gfni`, `lzcnt`, `movbe`, `pclmulqdq`, `popcnt`, `rdrand`, `rdseed`, `sha`, `vaes`, `vpclmulqdq`, `xsave`, `xsavec`, `xsaveopt`, `xsaves`"
);

// `Avx2` plus adx, aes, avx512bw, avx512cd, avx512dq, avx512f, avx512vl, pclmulqdq, rdrand,
// rdseed, xsavec, xsaveopt, xsaves (`avx2` and `f16c` are implied by `avx512f`).
token!(
    /// A token for the Skylake-SP feature set (`-march=skylake-avx512`, which equals x86-64-v4
    /// for vector code) on `x86` and `x86_64`: AVX-512 F, BW, CD, DQ and VL, as on Skylake-SP/X,
    /// Cascade Lake, Cooper Lake and every later AVX-512 CPU.
    ///
    /// Obtain it with [`Level::as_skylake_avx512`], then pass it to a function defined with
    /// [`kernel!`](crate::kernel) to call the AVX-512 F/BW/CD/DQ/VL intrinsics safely.
    SkylakeAvx512,
    "Skylake-SP",
    "fxsr,adx,aes,avx512bw,avx512cd,avx512dq,avx512f,avx512vl,bmi1,bmi2,cmpxchg16b,fma,lzcnt,movbe,pclmulqdq,popcnt,rdrand,rdseed,xsave,xsavec,xsaveopt,xsaves",
    "`adx`, `aes`, `avx512bw`, `avx512cd`, `avx512dq`, `avx512f`, `avx512vl`, `bmi1`, `bmi2`, `cmpxchg16b`, `fma`, `fxsr`, `lzcnt`, `movbe`, `pclmulqdq`, `popcnt`, `rdrand`, `rdseed`, `xsave`, `xsavec`, `xsaveopt`, `xsaves`"
);

impl Alderlake {
    /// The AVX2 token, for the portable vector types and AVX2-only kernels.
    #[inline]
    pub fn avx2(self) -> Avx2 {
        // Safety: the Alder Lake feature set includes the x86-64-v3 features required by Avx2.
        unsafe { Avx2::assume_supported() }
    }
}

impl SkylakeAvx512 {
    /// The AVX2 token, for the portable vector types and AVX2-only kernels.
    #[inline]
    pub fn avx2(self) -> Avx2 {
        // Safety: `avx512f` implies `avx2` and `f16c`, and the token has the rest of x86-64-v3.
        unsafe { Avx2::assume_supported() }
    }
}

// `Avx512` is `SkylakeAvx512` plus avx512bitalg, avx512ifma, avx512vbmi, avx512vbmi2, avx512vnni,
// avx512vpopcntdq, gfni, sha, vaes, vpclmulqdq.
impl Avx512 {
    /// The Skylake-SP token: the Ice Lake feature set includes it.
    #[inline]
    pub fn skylake_avx512(self) -> SkylakeAvx512 {
        // Safety: every feature required by SkylakeAvx512 is also required by Avx512.
        unsafe { SkylakeAvx512::assume_supported() }
    }
}

// Run-time detection on top of a proven x86-64-v3 level. `std` caches each query.
#[cfg(feature = "std")]
impl Avx2 {
    /// If the CPU also has the rest of the Alder Lake feature set, the token for it.
    ///
    /// Ice Lake and Zen 4 have no AVX-VNNI (only AVX512-VNNI), so this is `None` on them.
    #[inline]
    pub fn alderlake(self) -> Option<Alderlake> {
        if is_x86_feature_detected!("adx")
            && is_x86_feature_detected!("aes")
            && is_x86_feature_detected!("avxvnni")
            && is_x86_feature_detected!("gfni")
            && is_x86_feature_detected!("pclmulqdq")
            && is_x86_feature_detected!("rdrand")
            && is_x86_feature_detected!("rdseed")
            && is_x86_feature_detected!("sha")
            && is_x86_feature_detected!("vaes")
            && is_x86_feature_detected!("vpclmulqdq")
            && is_x86_feature_detected!("xsavec")
            && is_x86_feature_detected!("xsaveopt")
            && is_x86_feature_detected!("xsaves")
        {
            // Safety: `self` proves x86-64-v3 and the other features were detected above.
            Some(unsafe { Alderlake::assume_supported() })
        } else {
            None
        }
    }

    /// If the CPU also has the rest of the Skylake-SP feature set (AVX-512 F, BW, CD, DQ and
    /// VL with OS support for their state), the token for it.
    #[inline]
    pub fn skylake_avx512(self) -> Option<SkylakeAvx512> {
        if is_x86_feature_detected!("adx")
            && is_x86_feature_detected!("aes")
            && is_x86_feature_detected!("avx512bw")
            && is_x86_feature_detected!("avx512cd")
            && is_x86_feature_detected!("avx512dq")
            && is_x86_feature_detected!("avx512f")
            && is_x86_feature_detected!("avx512vl")
            && is_x86_feature_detected!("pclmulqdq")
            && is_x86_feature_detected!("rdrand")
            && is_x86_feature_detected!("rdseed")
            && is_x86_feature_detected!("xsavec")
            && is_x86_feature_detected!("xsaveopt")
            && is_x86_feature_detected!("xsaves")
        {
            // Safety: `self` proves x86-64-v3 and the other features were detected above.
            Some(unsafe { SkylakeAvx512::assume_supported() })
        } else {
            None
        }
    }
}

/// On an [`Avx2`] level the extra features are detected at run time, which needs the `std`
/// feature (without it these return `None` unless the level is [`Avx512`]).
impl Level {
    /// If this is a proof that x86-64-v3 is available and the CPU also has the Alder Lake
    /// feature set, the token for it.
    #[inline]
    pub fn as_alderlake(self) -> Option<Alderlake> {
        #[cfg(feature = "std")]
        if let Some(avx2) = self.as_avx2() {
            return avx2.alderlake();
        }
        None
    }

    /// If this is a proof that x86-64-v3 is available and the CPU also has the Skylake-SP
    /// feature set, the token for it. No detection is needed on an [`Avx512`] level.
    #[inline]
    pub fn as_skylake_avx512(self) -> Option<SkylakeAvx512> {
        if let Some(avx512) = self.as_avx512() {
            return Some(avx512.skylake_avx512());
        }
        #[cfg(feature = "std")]
        if let Some(avx2) = self.as_avx2() {
            return avx2.skylake_avx512();
        }
        None
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    #[cfg(target_arch = "x86")]
    use core::arch::x86::{
        __m256i, _mm256_gf2p8mul_epi8, _mm256_set1_epi8, _mm512_cmpeq_epi8_mask, _mm512_set1_epi8,
        _mm512_shuffle_epi8,
    };
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::{
        __m256i, _mm256_gf2p8mul_epi8, _mm256_set1_epi8, _mm512_cmpeq_epi8_mask, _mm512_set1_epi8,
        _mm512_shuffle_epi8,
    };
    use std::arch::is_x86_feature_detected;

    // 0x53 and 0xCA are inverses in GF(2^8) mod x^8 + x^4 + x^3 + x + 1 (FIPS-197).
    const A: i8 = 0x53_u8.cast_signed();
    const B: i8 = 0xCA_u8.cast_signed();

    crate::kernel!(
        fn gf_mul_256(_t: Alderlake, a: i8, b: i8) -> __m256i {
            _mm256_gf2p8mul_epi8(_mm256_set1_epi8(a), _mm256_set1_epi8(b))
        }
    );

    // Returns a lane-equality mask: bit i is set when byte i == `want`.
    crate::kernel!(
        fn shuffle_512(_t: SkylakeAvx512, table: i8, index: i8, want: i8) -> u64 {
            let picked = _mm512_shuffle_epi8(_mm512_set1_epi8(table), _mm512_set1_epi8(index));
            _mm512_cmpeq_epi8_mask(picked, _mm512_set1_epi8(want))
        }
    );

    #[test]
    fn tokens_match_detection() {
        let level = crate::Level::new();
        let avx2 = level.as_avx2().is_some();
        let common = is_x86_feature_detected!("adx")
            && is_x86_feature_detected!("aes")
            && is_x86_feature_detected!("pclmulqdq")
            && is_x86_feature_detected!("rdrand")
            && is_x86_feature_detected!("rdseed")
            && is_x86_feature_detected!("xsavec")
            && is_x86_feature_detected!("xsaveopt")
            && is_x86_feature_detected!("xsaves");
        let alderlake = is_x86_feature_detected!("avxvnni")
            && is_x86_feature_detected!("gfni")
            && is_x86_feature_detected!("sha")
            && is_x86_feature_detected!("vaes")
            && is_x86_feature_detected!("vpclmulqdq");
        let skylake_avx512 = is_x86_feature_detected!("avx512bw")
            && is_x86_feature_detected!("avx512cd")
            && is_x86_feature_detected!("avx512dq")
            && is_x86_feature_detected!("avx512f")
            && is_x86_feature_detected!("avx512vl");
        assert_eq!(
            level.as_alderlake().is_some(),
            avx2 && common && alderlake,
            "Alderlake"
        );
        assert_eq!(
            level.as_skylake_avx512().is_some(),
            avx2 && common && skylake_avx512,
            "SkylakeAvx512"
        );
    }

    #[test]
    fn kernel_instantiates_for_alderlake() {
        let Some(token) = crate::Level::new().as_alderlake() else {
            return;
        };
        let lanes: crate::u8x32<_> =
            crate::prelude::SimdInto::simd_into(gf_mul_256(token, A, B), token.avx2());
        assert_eq!(<[u8; 32]>::from(lanes), [1; 32], "256-bit GFNI multiply");
    }

    #[test]
    fn kernel_instantiates_for_skylake_avx512() {
        let Some(token) = crate::Level::new().as_skylake_avx512() else {
            return;
        };
        // Every table byte is 0x53, so any index picks 0x53.
        assert_eq!(
            shuffle_512(token, A, 1, A),
            u64::MAX,
            "512-bit byte shuffle"
        );
    }
}
