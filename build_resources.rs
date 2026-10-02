//! Minimal COFF resource object writer, used only during compilation.
//! Keeps GNU builds independent of an external Windows resource compiler.
use std::collections::BTreeMap;

fn word(out: &mut Vec<u8>, value: u16) { out.extend_from_slice(&value.to_le_bytes()); }
fn dword(out: &mut Vec<u8>, value: u32) { out.extend_from_slice(&value.to_le_bytes()); }
fn patch(out: &mut [u8], at: usize, value: u32) { out[at..at + 4].copy_from_slice(&value.to_le_bytes()); }
fn align(out: &mut Vec<u8>) { while out.len() % 4 != 0 { out.push(0); } }

fn directory(out: &mut Vec<u8>, count: usize) -> usize {
    let start = out.len();
    out.extend_from_slice(&[0; 14]);
    word(out, count.try_into().unwrap());
    out.resize(out.len() + count * 8, 0);
    start
}

pub fn write(icon: &[u8], manifest: &[u8], arch: &str) -> Vec<u8> {
    assert_eq!(&icon[..4], &[0, 0, 1, 0], "expected ICO file");
    let count = u16::from_le_bytes(icon[4..6].try_into().unwrap()) as usize;
    let mut resources: BTreeMap<u16, Vec<(u16, Vec<u8>)>> = BTreeMap::new();
    let mut group = icon[..6].to_vec();
    for i in 0..count {
        let entry = &icon[6 + i * 16..6 + (i + 1) * 16];
        let size = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
        let id = (i + 1) as u16;
        resources.entry(3).or_default().push((id, icon[offset..offset + size].to_vec()));
        group.extend_from_slice(&entry[..12]);
        word(&mut group, id);
    }
    resources.insert(14, vec![(1, group)]); // RT_GROUP_ICON
    resources.insert(24, vec![(1, manifest.to_vec())]); // RT_MANIFEST

    let mut section = Vec::new();
    let root = directory(&mut section, resources.len());
    let mut payloads = Vec::new();
    for (type_index, (kind, entries)) in resources.iter().enumerate() {
        let types = directory(&mut section, entries.len());
        patch(&mut section, root + 16 + type_index * 8, u32::from(*kind));
        patch(&mut section, root + 20 + type_index * 8, types as u32 | 0x8000_0000);
        for (index, (id, bytes)) in entries.iter().enumerate() {
            let languages = directory(&mut section, 1);
            patch(&mut section, types + 16 + index * 8, u32::from(*id));
            patch(&mut section, types + 20 + index * 8, languages as u32 | 0x8000_0000);
            patch(&mut section, languages + 16, 1033);
            let data_entry = section.len();
            patch(&mut section, languages + 20, data_entry as u32);
            section.extend_from_slice(&[0; 16]);
            patch(&mut section, data_entry + 4, bytes.len() as u32);
            payloads.push((data_entry, bytes));
        }
    }
    for (at, bytes) in &payloads {
        align(&mut section);
        let offset = section.len() as u32;
        patch(&mut section, *at, offset);
        section.extend_from_slice(bytes);
    }
    align(&mut section);

    let (machine, relocation) = match arch {
        "x86_64" => (0x8664, 0x0003), // AMD64_ADDR32NB
        "x86" => (0x014c, 0x0007), // I386_DIR32NB
        "aarch64" => (0xaa64, 0x0002), // ARM64_ADDR32NB
        _ => panic!("unsupported Windows resource architecture: {arch}"),
    };
    let reloc_start = 60 + section.len() as u32;
    let mut out = Vec::new();
    word(&mut out, machine);
    word(&mut out, 1); // one section
    dword(&mut out, 0); // deterministic timestamp
    dword(&mut out, reloc_start + payloads.len() as u32 * 10);
    dword(&mut out, 1); // one section symbol
    word(&mut out, 0); word(&mut out, 0);
    out.extend_from_slice(b".rsrc\0\0\0");
    dword(&mut out, 0); dword(&mut out, 0);
    dword(&mut out, section.len() as u32);
    dword(&mut out, 60);
    dword(&mut out, reloc_start);
    dword(&mut out, 0);
    word(&mut out, payloads.len() as u16); word(&mut out, 0);
    dword(&mut out, 0x4030_0040); // initialized data, read-only, 4-byte aligned
    out.extend_from_slice(&section);
    for (at, _) in payloads {
        dword(&mut out, at as u32);
        dword(&mut out, 0);
        word(&mut out, relocation);
    }
    out.extend_from_slice(b".rsrc\0\0\0");
    dword(&mut out, 0); word(&mut out, 1); word(&mut out, 0);
    out.extend_from_slice(&[3, 0]); // static symbol, no auxiliary records
    dword(&mut out, 4); // empty COFF string table
    out
}
