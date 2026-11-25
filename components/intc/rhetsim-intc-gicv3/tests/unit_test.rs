//! GICv3 单元测试

#[test]
fn test_gicd_new() {
    use rhetsim_intc_gicv3::gicd::GICD;
    let _ = GICD::new();
}
