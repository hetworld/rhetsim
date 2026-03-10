//! 外部网络接入接口（DPU nvme-of initiator、网络处理器等使用）
use crate::error::OmniResult;

#[derive(Debug, Clone)]
pub struct Packet {
    pub data: Vec<u8>,
    pub meta: PacketMeta,
}

#[derive(Debug, Clone, Default)]
pub struct PacketMeta {
    pub src_port: u16,
    pub dst_port: u16,
    pub proto:    u8,
}

pub trait NetworkBackend: Send + Sync {
    fn send_packet(&self, pkt: Packet) -> OmniResult<()>;
    fn recv_packet(&self) -> OmniResult<Option<Packet>>;
    fn connect(&mut self, addr: &str) -> OmniResult<()>;
    fn disconnect(&mut self) -> OmniResult<()>;
}
