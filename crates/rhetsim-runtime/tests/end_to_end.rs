//! End-to-end test: load Manifest → realize Platform → schedule execution

use rhetsim_core::backend::execution::BackendKind;
use rhetsim_core::unit::UnitCaps;
use rhetsim_cpu_mock::MockCpu;
use rhetsim_manifest::{load_from_str, ComponentRegistry, Realizer};
use rhetsim_runtime::Scheduler;
use std::sync::Arc;

fn mock_registry() -> Arc<ComponentRegistry> {
    let mut reg = ComponentRegistry::new();
    reg.register(
        "mock-cpu",
        Box::new(|desc| Ok(Box::new(MockCpu::new(&desc.id)))),
        None,
        &[BackendKind::Mock],
    );
    Arc::new(reg)
}

#[test]
fn load_manifest_and_realize() {
    let toml = r#"
        name = "test-platform"
        [[units]]
        id   = "cpu0"
        kind = "mock-cpu"
        [[units]]
        id   = "cpu1"
        kind = "mock-cpu"
    "#;

    let manifest = load_from_str(toml).expect("parse manifest");
    assert_eq!(manifest.name, "test-platform");
    assert_eq!(manifest.units.len(), 2);

    let registry = mock_registry();
    let realizer = Realizer::new(registry);
    let platform = realizer.realize(&manifest).expect("realize platform");

    assert_eq!(platform.unit_reg.all_ids().len(), 2);
}

#[test]
fn schedule_two_mock_cpus() {
    let toml = r#"
        name = "scheduler-test"
        [[units]]
        id   = "cpu0"
        kind = "mock-cpu"
        [[units]]
        id   = "cpu1"
        kind = "mock-cpu"
    "#;

    let manifest = load_from_str(toml).expect("parse manifest");
    let registry = mock_registry();
    let realizer = Realizer::new(registry);
    let platform = realizer.realize(&manifest).expect("realize platform");

    let executable = platform.unit_reg.find_by_caps(UnitCaps::EXECUTABLE);
    assert_eq!(executable.len(), 2, "should find 2 executable units");

    let mut scheduler = Scheduler::new(1_000_000);
    for unit in &executable {
        scheduler.add_unit(unit.clone());
    }

    let stats = scheduler.run(10_000_000, 1_000_000).expect("run scheduler");

    assert_eq!(stats.units_count, 2);
    assert_eq!(stats.quanta_executed, 10);
    // Each unit consumes full budget each quantum: 2 units × 10 quanta × 1_000_000 ns
    assert_eq!(stats.total_ns_elapsed, 20_000_000);
}

#[test]
fn load_minimal_toml_fixture() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/minimal.toml");

    let manifest = rhetsim_manifest::load_from_file(&path).expect("load minimal.toml");
    assert_eq!(manifest.name, "minimal-test");
    assert_eq!(manifest.units.len(), 2);
    assert_eq!(manifest.units[0].kind, "mock-cpu");

    let registry = mock_registry();
    let realizer = Realizer::new(registry);
    let platform = realizer.realize(&manifest).expect("realize from fixture");

    let mut scheduler = Scheduler::new(1_000_000);
    for unit in platform.unit_reg.executable_units() {
        scheduler.add_unit(unit);
    }

    let stats = scheduler.run_default(5_000_000).expect("run default");
    assert_eq!(stats.quanta_executed, 5);
    assert_eq!(stats.total_ns_elapsed, 10_000_000); // 2 units × 5 quanta × 1M ns
}
