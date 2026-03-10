//! 系统级测试入口 — 调用各子 crate 测试，验证跨层集成

#[cfg(test)]
mod tests {
    #[test]
    fn test_core_prelude_compiles() {
        use omnisim_core::prelude::*;
        let _ = UnitId::new("test.unit");
    }
}
