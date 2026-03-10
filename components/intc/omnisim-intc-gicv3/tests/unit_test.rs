//! GICv3 单元测试

#[test]
fn test_gicd_new() {
    use omnisim_intc_gicv3::gicd::GICD;
    let _ = GICD::new();
}
