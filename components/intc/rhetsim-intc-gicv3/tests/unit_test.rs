//! GICv3 unit tests

use rhetsim_intc_gicv3::Gicv3;

#[test]
fn test_gicv3_new() {
    let gic = Gicv3::new("gic0", 2, 64, 0x0800_0000);
    assert_eq!(gic.pending_irq(0), None);
    assert_eq!(gic.pending_irq(1), None);
}
