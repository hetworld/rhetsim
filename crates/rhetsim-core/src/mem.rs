//! MemTarget / MemSpace / AddressTranslator — 内存子系统接口

use crate::error::OmniResult;
use crate::security::MemSecurity;
use std::sync::Arc;

/// 内存访问粒度
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemAccessSize {
    /// 8 位访问
    U8 = 1,
    /// 16 位访问
    U16 = 2,
    /// 32 位访问
    U32 = 4,
    /// 64 位访问
    U64 = 8,
    /// 128 位访问
    U128 = 16,
}

/// 内存访问描述
#[derive(Debug, Clone, Copy)]
pub struct MemAccess {
    /// 访问地址
    pub addr: u64,
    /// 访问宽度
    pub size: MemAccessSize,
    /// 是否为写访问
    pub is_write: bool,
    /// 发起者 Unit ID（用于 MemBus visible_to 过滤）
    pub initiator_id: u32,
}

/// 访问类型（用于 AddressTranslator）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemAccessKind {
    /// 读访问
    Read,
    /// 写访问
    Write,
    /// 取指访问
    Execute,
}

/// 内存访问属性
#[derive(Debug, Clone, Copy, Default)]
pub struct MemAttrs {
    /// 可读
    pub readable: bool,
    /// 可写
    pub writable: bool,
    /// 可执行
    pub executable: bool,
    /// 可缓存
    pub cacheable: bool,
    /// 安全属性，默认 NonSecure
    pub security: MemSecurity,
}

impl MemAttrs {
    /// 可读写、不可执行、可缓存
    pub fn rw() -> Self {
        Self {
            readable: true,
            writable: true,
            executable: false,
            cacheable: true,
            security: MemSecurity::NonSecure,
        }
    }

    /// 只读、不可执行、可缓存
    pub fn ro() -> Self {
        Self {
            readable: true,
            writable: false,
            executable: false,
            cacheable: true,
            security: MemSecurity::NonSecure,
        }
    }

    /// 可读可执行、不可写、可缓存
    pub fn rx() -> Self {
        Self {
            readable: true,
            writable: false,
            executable: true,
            cacheable: true,
            security: MemSecurity::NonSecure,
        }
    }
}

/// 可被地址访问的单元 — 必须线程安全，read 不修改可观察状态
pub trait MemTarget: Send + Sync {
    /// realize 时由 MemBus 分配的唯一 ID
    fn target_id(&self) -> u32;

    /// 此 Target 覆盖的地址范围基址
    fn base(&self) -> u64;

    /// 地址范围大小
    fn size(&self) -> u64;

    /// 从目标地址空间读取
    fn read(&self, offset: u64, size: MemAccessSize) -> OmniResult<u64>;
    /// 向目标地址空间写入
    fn write(&self, offset: u64, size: MemAccessSize, val: u64) -> OmniResult<()>;

    /// 批量读（DMA / TCG 块操作优化路径）
    fn read_bulk(&self, offset: u64, buf: &mut [u8]) -> OmniResult<()> {
        for (i, b) in buf.iter_mut().enumerate() {
            *b = self.read(offset + i as u64, MemAccessSize::U8)? as u8;
        }
        Ok(())
    }

    /// 批量写
    fn write_bulk(&self, offset: u64, buf: &[u8]) -> OmniResult<()> {
        for (i, b) in buf.iter().enumerate() {
            self.write(offset + i as u64, MemAccessSize::U8, *b as u64)?;
        }
        Ok(())
    }

    /// 直接内存指针（RAM 优化：返回指针，避免拷贝；只有 RAM 类实现）
    fn as_raw_ptr(&self) -> Option<*mut u8> {
        None
    }
}

/// 地址转换接口（MMU / IOMMU 接入点）
pub trait AddressTranslator: Send + Sync {
    /// Stage-1：VA → IPA（Guest OS 页表，EL1 控制）
    fn translate_s1(&self, va: u64, asid: u16, access: MemAccessKind) -> OmniResult<u64>;

    /// Stage-2：IPA → PA（Hypervisor 页表，EL2 控制，P1 新增）
    fn translate_s2(&self, ipa: u64, vmid: u16, access: MemAccessKind) -> OmniResult<u64>;

    /// 两阶段级联：VA → IPA → PA
    fn translate_s1s2(
        &self,
        va: u64,
        asid: u16,
        vmid: u16,
        access: MemAccessKind,
    ) -> OmniResult<u64> {
        let ipa = self.translate_s1(va, asid, access)?;
        self.translate_s2(ipa, vmid, access)
    }

    /// TLB flush（VM 销毁或 VMID 回收）
    fn flush_tlb_vmid(&self, vmid: u16);

    /// TLB flush（进程切换）
    fn flush_tlb_asid(&self, asid: u16);
}

/// Mapping 内部记录
struct Mapping {
    target: Arc<dyn MemTarget>,
    host_base: u64,
    /// 预留：客户机侧映射基址，供后续地址转换使用。
    #[allow(dead_code)]
    guest_base: u64,
    size: u64,
    /// 预留：映射属性，供后续权限检查使用。
    #[allow(dead_code)]
    attrs: MemAttrs,
}

/// 地址空间：MemTarget 的路由表
/// 每个 Initiator Unit 持有自己独立的 MemSpace 实例
pub struct MemSpace {
    id: String,
    mappings: Vec<Mapping>,
    translator: Option<Box<dyn AddressTranslator>>,
}

impl MemSpace {
    /// 创建空的地址空间
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            mappings: Vec::new(),
            translator: None,
        }
    }

    /// 注册一个目标映射
    pub fn map(
        &mut self,
        target: Arc<dyn MemTarget>,
        host_base: u64,
        attrs: MemAttrs,
    ) -> OmniResult<()> {
        let guest_base = target.base();
        let size = target.size();
        self.mappings.push(Mapping {
            target,
            host_base,
            guest_base,
            size,
            attrs,
        });
        Ok(())
    }

    /// 移除指定宿主基址的映射
    pub fn unmap(&mut self, host_base: u64) -> OmniResult<()> {
        self.mappings.retain(|m| m.host_base != host_base);
        Ok(())
    }

    /// 从地址空间读取
    pub fn read(&self, addr: u64, size: MemAccessSize, _access: &MemAccess) -> OmniResult<u64> {
        let m = self.find(addr)?;
        m.target.read(addr - m.host_base, size)
    }

    /// 向地址空间写入
    pub fn write(
        &self,
        addr: u64,
        size: MemAccessSize,
        val: u64,
        _access: &MemAccess,
    ) -> OmniResult<()> {
        let m = self.find(addr)?;
        m.target.write(addr - m.host_base, size, val)
    }

    /// 设置地址转换器
    pub fn set_translator(&mut self, t: Box<dyn AddressTranslator>) {
        self.translator = Some(t);
    }

    fn find(&self, addr: u64) -> OmniResult<&Mapping> {
        self.mappings
            .iter()
            .find(|m| addr >= m.host_base && addr < m.host_base + m.size)
            .ok_or(crate::error::OmniError::AddressNotMapped(addr))
    }

    /// 地址空间 ID
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// CPU 特权级（P0 定义，P1 在 TCG 前端使用）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivilegeLevel {
    // ARM
    /// ARM EL0
    EL0,
    /// ARM EL1
    EL1,
    /// ARM EL2
    EL2,
    /// ARM EL3
    EL3,
    // x86
    /// x86 Ring 0
    Ring0,
    /// x86 Ring 1
    Ring1,
    /// x86 Ring 2
    Ring2,
    /// x86 Ring 3
    Ring3,
    /// VMX non-root（Guest）
    RingMinus1,
    // RISC-V
    /// RISC-V U-mode
    U,
    /// RISC-V S-mode
    S,
    /// RISC-V H-mode
    H,
    /// RISC-V M-mode
    M,
}
