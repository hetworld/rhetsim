//! RISC-V floating-point semantics shared by the interpreter and the JIT
//! helpers: NaN boxing, canonical-NaN generation, exception flags, rounding
//! modes and the integer conversions.
//!
//! Host IEEE exceptions and rounding modes come from the C `fenv` interface.
//! `fe*` is thread-local; each operation brackets itself with `begin`/`end`
//! and always restores round-to-nearest on exit.

/// Canonical quiet NaN bit patterns mandated by the F/D extension.
pub const CANONICAL_F32: u32 = 0x7FC0_0000;
pub const CANONICAL_F64: u64 = 0x7FF8_0000_0000_0000;
const NAN_BOX_MASK: u64 = 0xFFFF_FFFF_0000_0000;

/// `fcsr.fflags` bits.
pub const FFLAG_NX: u32 = 1 << 0;
pub const FFLAG_UF: u32 = 1 << 1;
pub const FFLAG_OF: u32 = 1 << 2;
pub const FFLAG_DZ: u32 = 1 << 3;
pub const FFLAG_NV: u32 = 1 << 4;

/// Resolve an instruction rounding field: 7 means "use frm".
pub fn effective_rm(rm: u8, frm: u8) -> u8 {
    if rm == 7 {
        frm & 0x7
    } else {
        rm
    }
}

#[cfg(target_os = "linux")]
mod host {
    // glibc x86_64 fenv values.
    pub const FE_INVALID: i32 = 0x01;
    pub const FE_DIVBYZERO: i32 = 0x04;
    pub const FE_OVERFLOW: i32 = 0x08;
    pub const FE_UNDERFLOW: i32 = 0x10;
    pub const FE_INEXACT: i32 = 0x20;
    pub const FE_ALL: i32 = FE_INVALID | FE_DIVBYZERO | FE_OVERFLOW | FE_UNDERFLOW | FE_INEXACT;
    pub const FE_TONEAREST: i32 = 0x000;
    pub const FE_DOWNWARD: i32 = 0x400;
    pub const FE_UPWARD: i32 = 0x800;
    pub const FE_TOWARDZERO: i32 = 0xC00;

    extern "C" {
        pub fn feclearexcept(excepts: i32) -> i32;
        pub fn fetestexcept(excepts: i32) -> i32;
        pub fn fesetround(round: i32) -> i32;
    }
}

/// Set the host rounding mode and clear pending exceptions.
///
/// RMM (rm=4) has no host equivalent and is approximated by round-to-nearest;
/// no selected riscv-tests case depends on it.
pub fn begin(rm: u8, frm: u8) {
    #[cfg(target_os = "linux")]
    unsafe {
        let mode = match effective_rm(rm, frm) {
            1 => host::FE_TOWARDZERO,
            2 => host::FE_DOWNWARD,
            3 => host::FE_UPWARD,
            _ => host::FE_TONEAREST,
        };
        host::fesetround(mode);
        host::feclearexcept(host::FE_ALL);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (rm, frm);
    }
}

/// Read the host exceptions raised since [`begin`], clear them and restore
/// round-to-nearest.
pub fn end() -> u32 {
    #[cfg(target_os = "linux")]
    unsafe {
        let host_flags = host::fetestexcept(host::FE_ALL);
        host::feclearexcept(host::FE_ALL);
        host::fesetround(host::FE_TONEAREST);
        let mut flags = 0;
        if host_flags & host::FE_INVALID != 0 {
            flags |= FFLAG_NV;
        }
        if host_flags & host::FE_DIVBYZERO != 0 {
            flags |= FFLAG_DZ;
        }
        if host_flags & host::FE_OVERFLOW != 0 {
            flags |= FFLAG_OF;
        }
        if host_flags & host::FE_UNDERFLOW != 0 {
            flags |= FFLAG_UF;
        }
        if host_flags & host::FE_INEXACT != 0 {
            flags |= FFLAG_NX;
        }
        flags
    }
    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

/// Restore round-to-nearest and clear pending exceptions without reporting.
pub fn reset_host() {
    #[cfg(target_os = "linux")]
    unsafe {
        host::feclearexcept(host::FE_ALL);
        host::fesetround(host::FE_TONEAREST);
    }
}

pub fn is_nan_f32(bits: u32) -> bool {
    bits & 0x7F80_0000 == 0x7F80_0000 && bits & 0x007F_FFFF != 0
}

pub fn is_nan_f64(bits: u64) -> bool {
    bits & 0x7FF0_0000_0000_0000 == 0x7FF0_0000_0000_0000 && bits & 0x000F_FFFF_FFFF_FFFF != 0
}

pub fn is_snan_f32(bits: u32) -> bool {
    is_nan_f32(bits) && bits & (1 << 22) == 0
}

pub fn is_snan_f64(bits: u64) -> bool {
    is_nan_f64(bits) && bits & (1 << 51) == 0
}

pub fn canonical_f32(value: f32) -> f32 {
    if value.is_nan() {
        f32::from_bits(CANONICAL_F32)
    } else {
        value
    }
}

pub fn canonical_f64(value: f64) -> f64 {
    if value.is_nan() {
        f64::from_bits(CANONICAL_F64)
    } else {
        value
    }
}

/// NaN-box a single-precision bit pattern for storage in an f-register.
pub fn nan_box(bits: u32) -> u64 {
    NAN_BOX_MASK | bits as u64
}

/// Effective single-precision bits: a register whose upper 32 bits are not
/// all ones is treated as the canonical NaN by single-precision operations.
pub fn single_bits(bits: u64) -> u32 {
    if bits & NAN_BOX_MASK == NAN_BOX_MASK {
        bits as u32
    } else {
        CANONICAL_F32
    }
}

pub fn single_value(bits: u64) -> f32 {
    f32::from_bits(single_bits(bits))
}

/// FMA shared by the interpreter and the JIT libcall. The caller sets the
/// rounding mode with [`begin`] and collects flags with [`end`].
pub fn fma_f32(a: f32, b: f32, c: f32) -> f32 {
    a.mul_add(b, c)
}

pub fn fma_f64(a: f64, b: f64, c: f64) -> f64 {
    a.mul_add(b, c)
}

fn round_to_integer(value: f64, rm: u8) -> f64 {
    match rm {
        1 => value.trunc(),
        2 => value.floor(),
        3 => value.ceil(),
        4 => value.round(), // RMM: ties away from zero
        _ => value.round_ties_even(),
    }
}

/// FP -> integer conversion with RISC-V saturation and flag rules.
/// `kind` is the rs2 field: 0=W, 1=WU, 2=L, 3=LU.
pub fn fcvt_to_int(value: f64, kind: u8, rm: u8, frm: u8) -> (u64, u32) {
    if value.is_nan() {
        // NaN converts to the largest positive value of the destination.
        let result = match kind {
            0 => (i32::MAX as i64) as u64,
            1 => ((u32::MAX as i32) as i64) as u64,
            2 => i64::MAX as u64,
            _ => u64::MAX,
        };
        return (result, FFLAG_NV);
    }
    let rm = effective_rm(rm, frm);
    let rounded = round_to_integer(value, rm);
    let inexact = if rounded != value { FFLAG_NX } else { 0 };

    match kind {
        0 => {
            if rounded >= 2_147_483_648.0 {
                return ((i32::MAX as i64) as u64, FFLAG_NV);
            }
            if rounded < -2_147_483_648.0 {
                return ((i32::MIN as i64) as u64, FFLAG_NV);
            }
            ((rounded as i32 as i64) as u64, inexact)
        }
        1 => {
            if rounded < 0.0 {
                return (0, FFLAG_NV);
            }
            if rounded >= 4_294_967_296.0 {
                // RV64 sign-extends the 32-bit unsigned result.
                return (((u32::MAX as i32) as i64) as u64, FFLAG_NV);
            }
            ((((rounded as u32) as i32) as i64) as u64, inexact)
        }
        2 => {
            if rounded >= 9_223_372_036_854_775_808.0 {
                return (i64::MAX as u64, FFLAG_NV);
            }
            if rounded < -9_223_372_036_854_775_808.0 {
                return (i64::MIN as u64, FFLAG_NV);
            }
            ((rounded as i64) as u64, inexact)
        }
        3 => {
            if rounded < 0.0 {
                return (0, FFLAG_NV);
            }
            if rounded >= 18_446_744_073_709_551_616.0 {
                return (u64::MAX, FFLAG_NV);
            }
            (rounded as u64, inexact)
        }
        _ => (0, FFLAG_NV),
    }
}

/// Integer -> FP conversion. Returns the f-register bits (NaN-boxed for a
/// single-precision result) and the exception flags.
///
/// The source is widened to `i128` so every RV64 integer source, including the
/// full unsigned 64-bit range, is represented exactly. Inexactness is judged
/// against the final destination width: converting 4294967294 to f32 is
/// inexact even though the intermediate f64 value is exact.
pub fn fcvt_from_int(value: u64, kind: u8, is_double: bool, rm: u8, frm: u8) -> (u64, u32) {
    begin(rm, frm);
    let integer: i128 = match kind {
        0 => value as i32 as i128,
        1 => value as u32 as i128,
        2 => value as i64 as i128,
        _ => value as u128 as i128,
    };
    let (bits, flags) = if is_double {
        let converted = integer as f64;
        let flags = if converted as i128 != integer {
            FFLAG_NX
        } else {
            0
        };
        (converted.to_bits(), flags)
    } else {
        let converted = integer as f32;
        let flags = if converted as i128 != integer {
            FFLAG_NX
        } else {
            0
        };
        (nan_box(converted.to_bits()), flags)
    };
    // RMM and non-RNE int->FP rounding remain approximated by the host cast;
    // `end` restores round-to-nearest and clears the host exception state.
    let _ = end();
    (bits, flags)
}

/// FCVT.S.D / FCVT.D.S conversion with canonical NaN and sNaN flags.
pub fn fcvt_fmt(bits: u64, from_double: bool, to_double: bool, rm: u8, frm: u8) -> (u64, u32) {
    if from_double {
        let value = f64::from_bits(bits);
        if value.is_nan() {
            let flags = if is_snan_f64(bits) { FFLAG_NV } else { 0 };
            let out = if to_double {
                CANONICAL_F64
            } else {
                nan_box(CANONICAL_F32)
            };
            return (out, flags);
        }
        begin(rm, frm);
        let single = value as f32;
        let flags = end();
        let single = canonical_f32(single);
        if to_double {
            ((single as f64).to_bits(), flags)
        } else {
            (nan_box(single.to_bits()), flags)
        }
    } else {
        let source_bits = single_bits(bits);
        let value = f32::from_bits(source_bits);
        if value.is_nan() {
            let flags = if is_snan_f32(source_bits) {
                FFLAG_NV
            } else {
                0
            };
            let out = if to_double {
                CANONICAL_F64
            } else {
                nan_box(CANONICAL_F32)
            };
            return (out, flags);
        }
        begin(rm, frm);
        let double = value as f64;
        let flags = end();
        (double.to_bits(), flags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nan_boxing() {
        assert_eq!(single_bits(0xFFFF_FFFF_7FC0_0000), 0x7FC0_0000);
        assert_eq!(single_bits(0x7FFF_FFFF_1234_5678), CANONICAL_F32);
    }

    #[test]
    fn fcvt_to_int_rounding_and_flags() {
        assert_eq!(fcvt_to_int(-1.1, 0, 1, 0), ((-1i64) as u64, FFLAG_NX));
        assert_eq!(fcvt_to_int(-1.0, 0, 1, 0), ((-1i64) as u64, 0));
        assert_eq!(fcvt_to_int(-0.9, 1, 1, 0), (0, FFLAG_NX));
        assert_eq!(fcvt_to_int(3.0e9, 0, 1, 0), (i32::MAX as u64, FFLAG_NV));
        assert_eq!(
            fcvt_to_int(5.0e9, 1, 1, 0),
            (((u32::MAX as i32) as i64) as u64, FFLAG_NV)
        );
        assert_eq!(
            fcvt_to_int(f64::NAN, 0, 1, 0),
            ((i32::MAX as i64) as u64, FFLAG_NV)
        );
        assert_eq!(
            fcvt_to_int(f64::INFINITY, 2, 1, 0),
            (i64::MAX as u64, FFLAG_NV)
        );
    }

    #[test]
    fn fcvt_wu_mid_range() {
        let value = 3.0e9f32 as f64;
        let (bits, flags) = fcvt_to_int(value, 1, 1, 0);
        assert_eq!(bits, 0xFFFF_FFFF_B2D0_5E00);
        assert_eq!(flags, 0);
    }

    #[test]
    fn canonical_nan_conversions() {
        // 0x7ffcffffffff8004 is a quiet NaN: canonicalized without NV.
        let (bits, flags) = fcvt_fmt(0x7FFC_FFFF_FFFF_8004, true, false, 0, 0);
        assert_eq!(bits, nan_box(CANONICAL_F32));
        assert_eq!(flags, 0);
        // A signaling NaN reports invalid while converting to canonical.
        let (bits, flags) = fcvt_fmt(0x7FF0_0000_0000_0001, true, false, 0, 0);
        assert_eq!(bits, nan_box(CANONICAL_F32));
        assert_eq!(flags, FFLAG_NV);
        let (bits, flags) = fcvt_fmt(nan_box(CANONICAL_F32), false, true, 0, 0);
        assert_eq!(bits, CANONICAL_F64);
        assert_eq!(flags, 0);
    }
}
