//! GICv3 interrupt controller (Distributor + Redistributor + CPU Interface)
pub mod gicc;
pub mod gicd;
pub mod gicr;
pub mod gicv3;

pub use gicv3::Gicv3;

pub fn register() {}
