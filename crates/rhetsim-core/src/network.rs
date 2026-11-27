//! 外部网络接入接口（DPU nvme-of initiator、网络处理器等使用）
use crate::error::OmniResult;

/// 一个网络数据包
#[derive(Debug, Clone)]
pub struct Packet {
    /// 包负载
    pub data: Vec<u8>,
    /// 包元数据
    pub meta: PacketMeta,
}

/// 网络包元数据
#[derive(Debug, Clone, Default)]
pub struct PacketMeta {
    /// 源端口
    pub src_port: u16,
    /// 目的端口
    pub dst_port: u16,
    /// 协议号
    pub proto: u8,
}

/// 外部网络后端
pub trait NetworkBackend: Send + Sync {
    /// 发送一个数据包
    fn send_packet(&self, pkt: Packet) -> OmniResult<()>;
    /// 接收一个数据包；无数据时返回 `None`
    fn recv_packet(&self) -> OmniResult<Option<Packet>>;
    /// 连接到远端地址
    fn connect(&mut self, addr: &str) -> OmniResult<()>;
    /// 断开连接
    fn disconnect(&mut self) -> OmniResult<()>;
}
