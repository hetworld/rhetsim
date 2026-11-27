//! RISC-V CPU profiles.
//!
//! A profile separates what a real CPU declares from what this simulator
//! currently implements. Public profiles (SiFive U74 / T-Head C908) are
//! registered as alignment targets; only profiles whose declared extensions
//! are all implemented may be used to boot a guest.

/// ISA extension bits used by CPU profiles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IsaExtensions(u64);

impl IsaExtensions {
    pub const NONE: Self = Self(0);
    pub const I: Self = Self(1 << 0);
    pub const M: Self = Self(1 << 1);
    pub const A: Self = Self(1 << 2);
    pub const F: Self = Self(1 << 3);
    pub const D: Self = Self(1 << 4);
    pub const C: Self = Self(1 << 5);
    pub const V: Self = Self(1 << 6);
    pub const S: Self = Self(1 << 7);
    pub const U: Self = Self(1 << 8);
    pub const ZICSR: Self = Self(1 << 9);
    pub const ZIFENCEI: Self = Self(1 << 10);
    pub const SSTC: Self = Self(1 << 11);
    pub const ZBA: Self = Self(1 << 12);
    pub const ZBB: Self = Self(1 << 13);
    pub const ZBC: Self = Self(1 << 14);
    pub const ZBS: Self = Self(1 << 15);
    pub const ZBKC: Self = Self(1 << 16);
    pub const ZKT: Self = Self(1 << 17);
    pub const ZICBOM: Self = Self(1 << 18);
    pub const ZICBOZ: Self = Self(1 << 19);
    pub const ZIHINTPAUSE: Self = Self(1 << 20);
    pub const SVINVAL: Self = Self(1 << 21);
    pub const SVNAPOT: Self = Self(1 << 22);
    pub const SVPBMT: Self = Self(1 << 23);
    pub const SSCOFPMF: Self = Self(1 << 24);
    pub const SMEPMP: Self = Self(1 << 25);
    pub const XTHEAD: Self = Self(1 << 26);
    pub const XTHEAD_SYNC: Self = Self(1 << 27);

    /// Set union, usable in constant profile definitions.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// True when every bit in `other` is present.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Remove every bit in `other`.
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// `misa` value for the single-letter extensions in this set.
    pub const fn misa(self) -> u64 {
        let mut bits = 2u64 << 62; // MXL=64
        if self.contains(Self::A) {
            bits |= 1 << 0;
        }
        if self.contains(Self::C) {
            bits |= 1 << 2;
        }
        if self.contains(Self::D) {
            bits |= 1 << 3;
        }
        if self.contains(Self::F) {
            bits |= 1 << 5;
        }
        if self.contains(Self::I) {
            bits |= 1 << 8;
        }
        if self.contains(Self::M) {
            bits |= 1 << 12;
        }
        if self.contains(Self::S) {
            bits |= 1 << 18;
        }
        if self.contains(Self::U) {
            bits |= 1 << 20;
        }
        if self.contains(Self::V) {
            bits |= 1 << 21;
        }
        bits
    }

    /// Human-readable extension names, used in validation errors.
    pub fn names(self) -> Vec<&'static str> {
        const NAMES: &[(u64, &str)] = &[
            (1 << 0, "i"),
            (1 << 1, "m"),
            (1 << 2, "a"),
            (1 << 3, "f"),
            (1 << 4, "d"),
            (1 << 5, "c"),
            (1 << 6, "v"),
            (1 << 7, "s"),
            (1 << 8, "u"),
            (1 << 9, "zicsr"),
            (1 << 10, "zifencei"),
            (1 << 11, "sstc"),
            (1 << 12, "zba"),
            (1 << 13, "zbb"),
            (1 << 14, "zbc"),
            (1 << 15, "zbs"),
            (1 << 16, "zbkc"),
            (1 << 17, "zkt"),
            (1 << 18, "zicbom"),
            (1 << 19, "zicboz"),
            (1 << 20, "zihintpause"),
            (1 << 21, "svinval"),
            (1 << 22, "svnapot"),
            (1 << 23, "svpbmt"),
            (1 << 24, "sscofpmf"),
            (1 << 25, "smepmp"),
            (1 << 26, "xthead"),
            (1 << 27, "xthead-sync"),
        ];
        NAMES
            .iter()
            .filter(|(bit, _)| self.0 & bit != 0)
            .map(|(_, name)| *name)
            .collect()
    }
}

/// Immutable description of one CPU type.
#[derive(Clone, Copy, Debug)]
pub struct CpuProfile {
    pub name: &'static str,
    pub vendor: &'static str,
    /// Extensions the real CPU advertises.
    pub declared: IsaExtensions,
    /// Extensions this simulator currently implements for the profile.
    pub implemented: IsaExtensions,
    pub dts_isa: &'static str,
    pub dts_compatible: &'static [&'static str],
    pub mvendorid: u64,
    pub marchid: u64,
    pub mimpid: u64,
}

impl CpuProfile {
    /// Missing extensions that block using this profile to boot a guest.
    pub fn missing(&self) -> IsaExtensions {
        self.declared.difference(self.implemented)
    }

    pub fn is_implemented(&self) -> bool {
        self.missing().is_empty()
    }

    pub fn misa(&self) -> u64 {
        self.implemented.misa()
    }
}

const COMMON_IMPLEMENTED: IsaExtensions = IsaExtensions::I
    .union(IsaExtensions::M)
    .union(IsaExtensions::A)
    .union(IsaExtensions::S)
    .union(IsaExtensions::U)
    .union(IsaExtensions::ZICSR)
    .union(IsaExtensions::ZIFENCEI);

const RHETSIM_IMA_IMPLEMENTED: IsaExtensions = COMMON_IMPLEMENTED.union(IsaExtensions::SSTC);
const RHETSIM_IMAC_IMPLEMENTED: IsaExtensions = RHETSIM_IMA_IMPLEMENTED.union(IsaExtensions::C);

const RHETSIM_IMA_PROFILE: CpuProfile = CpuProfile {
    name: "rhetsim-rv64ima",
    vendor: "RhetSim",
    declared: RHETSIM_IMA_IMPLEMENTED,
    implemented: RHETSIM_IMA_IMPLEMENTED,
    dts_isa: "rv64ima",
    dts_compatible: &["rhetsim,rv64ima"],
    mvendorid: 0,
    marchid: 0,
    mimpid: 0,
};

const RHETSIM_IMAC_PROFILE: CpuProfile = CpuProfile {
    name: "rhetsim-rv64imac",
    vendor: "RhetSim",
    declared: RHETSIM_IMAC_IMPLEMENTED,
    implemented: RHETSIM_IMAC_IMPLEMENTED,
    dts_isa: "rv64imac",
    dts_compatible: &["rhetsim,rv64imac"],
    mvendorid: 0,
    marchid: 0,
    mimpid: 0,
};

/// SiFive U74: RV64GC_Zba_Zbb_Sscofpmf, Sv39, M/S/U.
const SIFIVE_U74_PROFILE: CpuProfile = CpuProfile {
    name: "sifive-u74",
    vendor: "SiFive",
    declared: IsaExtensions::I
        .union(IsaExtensions::M)
        .union(IsaExtensions::A)
        .union(IsaExtensions::F)
        .union(IsaExtensions::D)
        .union(IsaExtensions::C)
        .union(IsaExtensions::S)
        .union(IsaExtensions::U)
        .union(IsaExtensions::ZICSR)
        .union(IsaExtensions::ZIFENCEI)
        .union(IsaExtensions::ZBA)
        .union(IsaExtensions::ZBB)
        .union(IsaExtensions::SSCOFPMF),
    implemented: COMMON_IMPLEMENTED
        .union(IsaExtensions::C)
        .union(IsaExtensions::F)
        .union(IsaExtensions::D)
        .union(IsaExtensions::ZBA)
        .union(IsaExtensions::ZBB)
        .union(IsaExtensions::SSCOFPMF),
    dts_isa: "rv64gc_zba_zbb_sscofpmf",
    dts_compatible: &["sifive,u74"],
    mvendorid: 0x489,
    marchid: 0,
    mimpid: 0,
};

/// T-Head XuanTie C908 (base, without the optional Vector unit).
const THEAD_C908_PROFILE: CpuProfile = CpuProfile {
    name: "thead-c908",
    vendor: "T-Head",
    declared: IsaExtensions::I
        .union(IsaExtensions::M)
        .union(IsaExtensions::A)
        .union(IsaExtensions::F)
        .union(IsaExtensions::D)
        .union(IsaExtensions::C)
        .union(IsaExtensions::S)
        .union(IsaExtensions::U)
        .union(IsaExtensions::ZICSR)
        .union(IsaExtensions::ZIFENCEI)
        .union(IsaExtensions::SSTC)
        .union(IsaExtensions::ZBA)
        .union(IsaExtensions::ZBB)
        .union(IsaExtensions::ZBC)
        .union(IsaExtensions::ZBS)
        .union(IsaExtensions::ZBKC)
        .union(IsaExtensions::ZKT)
        .union(IsaExtensions::ZICBOM)
        .union(IsaExtensions::ZICBOZ)
        .union(IsaExtensions::ZIHINTPAUSE)
        .union(IsaExtensions::SVINVAL)
        .union(IsaExtensions::SVNAPOT)
        .union(IsaExtensions::SVPBMT)
        .union(IsaExtensions::SSCOFPMF)
        .union(IsaExtensions::SMEPMP)
        .union(IsaExtensions::XTHEAD)
        .union(IsaExtensions::XTHEAD_SYNC),
    implemented: COMMON_IMPLEMENTED
        .union(IsaExtensions::C)
        .union(IsaExtensions::F)
        .union(IsaExtensions::D)
        .union(IsaExtensions::ZBA)
        .union(IsaExtensions::ZBB)
        .union(IsaExtensions::ZBC)
        .union(IsaExtensions::ZBS)
        .union(IsaExtensions::ZBKC)
        .union(IsaExtensions::ZKT)
        .union(IsaExtensions::ZIHINTPAUSE)
        .union(IsaExtensions::ZICBOM)
        .union(IsaExtensions::ZICBOZ)
        .union(IsaExtensions::SVINVAL)
        .union(IsaExtensions::SVNAPOT)
        .union(IsaExtensions::SVPBMT)
        .union(IsaExtensions::SSTC)
        .union(IsaExtensions::SSCOFPMF)
        .union(IsaExtensions::XTHEAD_SYNC),
    dts_isa: "rv64gcb",
    dts_compatible: &["thead,c908"],
    mvendorid: 0x5b7,
    marchid: 0x8d143000,
    mimpid: 0,
};

/// Supported CPU type selectors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RiscvCpuType {
    /// RhetSim's minimal custom profile without compressed instructions.
    RhetSimRv64Ima,
    /// RhetSim's current, fully implemented Linux profile.
    #[default]
    RhetSimRv64Imac,
    /// SiFive U74 alignment profile (F/D and bitmanip pending).
    SiFiveU74,
    /// T-Head C908 alignment profile (F/D, bitmanip and vendor extensions pending).
    THeadC908,
}

impl RiscvCpuType {
    pub const ALL: &'static [Self] = &[
        Self::RhetSimRv64Ima,
        Self::RhetSimRv64Imac,
        Self::SiFiveU74,
        Self::THeadC908,
    ];

    pub fn parse(value: &str) -> Result<Self, String> {
        let value = value.trim();
        if value.eq_ignore_ascii_case("rhetsim-rv64ima") || value.eq_ignore_ascii_case("rv64ima") {
            Ok(Self::RhetSimRv64Ima)
        } else if value.eq_ignore_ascii_case("rhetsim-rv64imac")
            || value.eq_ignore_ascii_case("rv64imac")
        {
            Ok(Self::RhetSimRv64Imac)
        } else if value.eq_ignore_ascii_case("sifive-u74") || value.eq_ignore_ascii_case("u74") {
            Ok(Self::SiFiveU74)
        } else if value.eq_ignore_ascii_case("thead-c908")
            || value.eq_ignore_ascii_case("xuantie-c908")
            || value.eq_ignore_ascii_case("c908")
        {
            Ok(Self::THeadC908)
        } else {
            Err(format!(
                "unknown RISC-V CPU type '{value}' (known: {})",
                Self::ALL
                    .iter()
                    .map(|t| t.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        }
    }

    pub const fn profile(self) -> &'static CpuProfile {
        match self {
            Self::RhetSimRv64Ima => &RHETSIM_IMA_PROFILE,
            Self::RhetSimRv64Imac => &RHETSIM_IMAC_PROFILE,
            Self::SiFiveU74 => &SIFIVE_U74_PROFILE,
            Self::THeadC908 => &THEAD_C908_PROFILE,
        }
    }

    pub const fn name(self) -> &'static str {
        self.profile().name
    }

    pub fn has_compressed(self) -> bool {
        self.profile().implemented.contains(IsaExtensions::C)
    }

    pub fn misa(self) -> u64 {
        self.profile().misa()
    }

    /// Reject a profile whose declared extensions are not all implemented.
    pub fn ensure_implemented(self) -> Result<(), String> {
        let missing = self.profile().missing();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "CPU type '{}' declares unimplemented extensions: {}",
                self.name(),
                missing.names().join(", ")
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_profiles_are_complete() {
        assert!(RiscvCpuType::RhetSimRv64Ima.ensure_implemented().is_ok());
        assert!(RiscvCpuType::RhetSimRv64Imac.ensure_implemented().is_ok());
    }

    #[test]
    fn public_profiles_report_missing_features() {
        assert!(RiscvCpuType::SiFiveU74.ensure_implemented().is_ok());
        assert!(RiscvCpuType::SiFiveU74
            .profile()
            .implemented
            .contains(IsaExtensions::F));
        assert!(RiscvCpuType::SiFiveU74
            .profile()
            .implemented
            .contains(IsaExtensions::D));

        let c908 = RiscvCpuType::THeadC908.profile().missing();
        assert!(c908.contains(IsaExtensions::XTHEAD));
        assert!(c908.contains(IsaExtensions::SMEPMP));
        assert!(!c908.contains(IsaExtensions::XTHEAD_SYNC));
        assert!(!c908.contains(IsaExtensions::F));
        assert!(!c908.contains(IsaExtensions::D));
        assert!(!c908.contains(IsaExtensions::ZBC));
        assert!(!c908.contains(IsaExtensions::ZBS));
        assert!(!c908.contains(IsaExtensions::SVINVAL));
        assert!(!c908.contains(IsaExtensions::SVNAPOT));
        assert!(!c908.contains(IsaExtensions::SVPBMT));
        // Vector is optional on C908 and is not part of the base profile.
        assert!(!RiscvCpuType::THeadC908
            .profile()
            .declared
            .contains(IsaExtensions::V));
    }

    #[test]
    fn imac_misa_contains_compressed() {
        let misa = RiscvCpuType::RhetSimRv64Imac.misa();
        assert!(misa & (1 << 2) != 0); // C
        assert!(misa & (1 << 8) != 0); // I
        assert!(misa & (1 << 12) != 0); // M
        assert!(misa & (1 << 0) != 0); // A
    }

    #[test]
    fn ima_misa_omits_compressed() {
        let misa = RiscvCpuType::RhetSimRv64Ima.misa();
        assert_eq!(misa & (1 << 2), 0); // C
    }

    #[test]
    fn parse_aliases() {
        assert_eq!(
            RiscvCpuType::parse("rv64imac").unwrap(),
            RiscvCpuType::RhetSimRv64Imac
        );
        assert_eq!(
            RiscvCpuType::parse("sifive-u74").unwrap(),
            RiscvCpuType::SiFiveU74
        );
        assert_eq!(
            RiscvCpuType::parse("c908").unwrap(),
            RiscvCpuType::THeadC908
        );
        assert!(RiscvCpuType::parse("rv64gcv").is_err());
    }
}
