//! DataEndpoint / DataDesc / DataOp — 高带宽数据流接口
//!
//! 用于 NPU/DPU 等加速器与内存之间的大块数据传输，
//! 与 MemBus 分离以避免 MMIO 路径的开销。

use crate::error::OmniResult;

/// 数据传输令牌（异步操作句柄）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DataToken(pub u64);

/// 数据传输结果
#[derive(Debug)]
pub struct DataResult {
    pub token:    DataToken,
    pub bytes:    usize,
    pub success:  bool,
}

/// 数据流端点（生产者或消费者）
pub trait DataEndpoint: Send + Sync {
    fn endpoint_id(&self) -> u32;

    /// 生产者：提交一批数据传输请求
    fn produce(&self, desc: DataDesc) -> OmniResult<DataToken>;

    /// 消费者：取回处理结果
    fn consume(&self, token: DataToken) -> OmniResult<DataResult>;

    /// 轮询：是否有数据/结果就绪
    fn poll_ready(&self) -> bool;
}

/// 数据传输描述符
#[derive(Debug)]
pub struct DataDesc {
    /// 源地址（在 Initiator 的 MemSpace 中）
    pub src_addr: u64,
    /// 目标地址
    pub dst_addr: u64,
    pub len:      usize,
    pub op:       DataOp,
    /// 算子参数（NPU）/ 数据包头（DPU）/ 自定义
    pub metadata: Vec<u8>,
}

/// 数据操作类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataOp {
    DmaRead,
    DmaWrite,
    DmaCopy,
    /// NPU 算子推理
    NpuInference,
    /// DPU 数据包处理
    DpuPacketProcess,
    Custom(u32),
}
