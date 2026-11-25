mod debugger;

fn main() {
    eprintln!("rhetsim-dbg: interactive debugger");
    eprintln!("Usage: attach to a running simulation via QMP, or use as a library");
    eprintln!("Available commands: step, continue, break <addr>, delete <addr>, info regs, x/<n> <addr>, quit");
}
