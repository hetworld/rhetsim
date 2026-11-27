//! Shared helpers for the `riscv-tests` integration tests.
#![allow(dead_code)]

use object::{Object, ObjectSegment, ObjectSymbol};
use std::path::Path;

pub const MEM_BASE: u64 = 0x8000_0000;
pub const MEM_SIZE: usize = 16 * 1024 * 1024;

pub struct Image {
    pub entry: u64,
    pub segments: Vec<(u64, Vec<u8>)>,
    pub tohost: u64,
    pub write_tohost: Option<u64>,
    pub signature: Option<(u64, u64)>,
}

pub fn parse_image(path: &Path) -> Image {
    let data = std::fs::read(path).expect("read ELF");
    let file = object::File::parse(&*data).expect("parse ELF");

    let mut segments = Vec::new();
    for segment in file.segments() {
        let bytes = segment.data().unwrap_or(&[]);
        if bytes.is_empty() {
            continue;
        }
        let addr = segment.address();
        assert!(addr >= MEM_BASE, "segment below DRAM: {addr:#x}");
        segments.push((addr - MEM_BASE, bytes.to_vec()));
    }

    let mut tohost = None;
    let mut write_tohost = None;
    let mut begin_signature = None;
    let mut end_signature = None;
    for symbol in file.symbols() {
        match symbol.name() {
            Ok("tohost") => tohost = Some(symbol.address()),
            Ok("write_tohost") => write_tohost = Some(symbol.address()),
            Ok("begin_signature") => begin_signature = Some(symbol.address()),
            Ok("end_signature") => end_signature = Some(symbol.address()),
            _ => {}
        }
    }

    Image {
        entry: file.entry(),
        segments,
        tohost: tohost.expect("tohost symbol"),
        write_tohost,
        signature: begin_signature.zip(end_signature),
    }
}

/// `riscv-tests` p-test binaries are shipped without an extension suffix.
pub fn is_riscv_ptest(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    path.is_file() && name.starts_with("rv64") && name.contains("-p-") && !name.contains('.')
}
