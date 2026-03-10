//! omni-cli — OmniSim 命令行入口，注册所有组件

fn main() {
    tracing_subscriber::fmt::init();

    // 注册所有组件
    omnisim_cpu_arm::register();
    omnisim_cpu_riscv::register();
    omnisim_cpu_x86::register();
    omnisim_cpu_mock::register();
    omnisim_intc_gicv3::register();
    omnisim_intc_plic::register();
    omnisim_intc_clint::register();
    omnisim_cache::register();
    omnisim_mem_dram::register();
    omnisim_mem_sram::register();
    omnisim_mem_gddr::register();
    omnisim_io_uart::register();
    omnisim_io_pcie::register();
    omnisim_io_dma::register();
    omnisim_io_mailbox::register();
    omnisim_io_virtio::register();
    omnisim_accel_gpgpu::register();
    omnisim_accel_dpu::register();

    tracing::info!("OmniSim all components registered");
}
