use crate::util::make_executable;
use goblin::mach::Mach;
use std::path::{Path, PathBuf};

const FAT_MAGIC: u32 = 0xcafe_babe;
const CPU_TYPE_ARM: u32 = 12;
const CPU_TYPE_ARM64: u32 = 0x0100_000c;
const CPU_TYPE_ARM64_32: u32 = 0x0200_000c;

struct Slice {
    bytes: Vec<u8>,
    cputype: u32,
    cpusubtype: u32,
    align: u32,
}

pub fn merge(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
    let mut slices: Vec<Slice> = Vec::new();
    for input in inputs {
        let bytes = std::fs::read(input).map_err(|e| format!("{}: {e}", input.display()))?;
        let (cputype, cpusubtype) = match Mach::parse(&bytes).map_err(|e| format!("{}: {e}", input.display()))? {
            Mach::Binary(macho) => (macho.header.cputype, macho.header.cpusubtype),
            Mach::Fat(_) => return Err(format!("{} is already a universal binary", input.display())),
        };
        if slices.iter().any(|slice| slice.cputype == cputype) {
            return Err(format!("{} repeats an architecture already in the universal binary", input.display()));
        }
        let align = if matches!(cputype, CPU_TYPE_ARM | CPU_TYPE_ARM64 | CPU_TYPE_ARM64_32) { 14 } else { 12 };
        slices.push(Slice { bytes, cputype, cpusubtype, align });
    }
    slices.sort_by_key(|slice| slice.align);

    let mut offsets = Vec::with_capacity(slices.len());
    let mut end = 8 + 20 * slices.len() as u64;
    for slice in &slices {
        let offset = end.next_multiple_of(1 << slice.align);
        offsets.push(offset);
        end = offset + slice.bytes.len() as u64;
    }
    if end > u64::from(u32::MAX) {
        return Err(format!("{} would be larger than a universal binary can address", output.display()));
    }

    let mut fat = Vec::with_capacity(end as usize);
    fat.extend_from_slice(&FAT_MAGIC.to_be_bytes());
    fat.extend_from_slice(&(slices.len() as u32).to_be_bytes());
    for (slice, offset) in slices.iter().zip(&offsets) {
        for field in [slice.cputype, slice.cpusubtype, *offset as u32, slice.bytes.len() as u32, slice.align] {
            fat.extend_from_slice(&field.to_be_bytes());
        }
    }
    for (slice, offset) in slices.iter().zip(&offsets) {
        fat.resize(*offset as usize, 0);
        fat.extend_from_slice(&slice.bytes);
    }

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(output, fat).map_err(|e| format!("{}: {e}", output.display()))?;
    make_executable(output)
}
