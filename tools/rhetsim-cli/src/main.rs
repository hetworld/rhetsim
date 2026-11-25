//! rhetsim-cli — RhetSim 命令行入口，注册所有组件

fn main() {
    tracing_subscriber::fmt::init();

    // 注册所有组件
    rhetsim_cpu_arm::register();
    rhetsim_cpu_riscv::register();
    rhetsim_cpu_x86::register();
    rhetsim_cpu_mock::register();
    rhetsim_intc_gicv3::register();
    rhetsim_intc_plic::register();
    rhetsim_intc_clint::register();
    rhetsim_cache::register();
    rhetsim_mem_dram::register();
    rhetsim_mem_sram::register();
    rhetsim_mem_gddr::register();
    rhetsim_io_uart::register();
    rhetsim_io_pcie::register();
    rhetsim_io_dma::register();
    rhetsim_io_mailbox::register();
    rhetsim_io_virtio::register();
    rhetsim_accel_gpgpu::register();
    rhetsim_accel_dpu::register();

    tracing::info!("RhetSim all components registered");
}
