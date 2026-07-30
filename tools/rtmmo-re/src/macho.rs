use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Context, Result};
use object::read::macho::{FatArch, LoadCommandVariant, MachOFatFile32, MachOFatFile64};
use object::{Architecture, FileKind, Object, ObjectSection, ObjectSymbol, SymbolScope};
use sha2::{Digest, Sha256};

use crate::model::MachOInfo;
use crate::{codesign, objc, redact};

pub fn inspect(path: &str, bytes: &[u8]) -> Result<MachOInfo> {
    let kind = FileKind::parse(bytes)
        .map_err(|error| anyhow!("input is not a Mach-O image ({path}): {error}"))?;
    let mut info = match kind {
        FileKind::MachO32 | FileKind::MachO64 => inspect_thin(path, bytes)?,
        FileKind::MachOFat32 => {
            let fat = MachOFatFile32::parse(bytes).context("parse 32-bit fat Mach-O header")?;
            inspect_single_fat_arch(path, bytes, fat.arches())?
        }
        FileKind::MachOFat64 => {
            let fat = MachOFatFile64::parse(bytes).context("parse 64-bit fat Mach-O header")?;
            inspect_single_fat_arch(path, bytes, fat.arches())?
        }
        _ => bail!("input is not a Mach-O image: {path}"),
    };
    info.sha256 = sha256(bytes);
    Ok(info)
}

fn inspect_thin(path: &str, bytes: &[u8]) -> Result<MachOInfo> {
    let file = object::File::parse(bytes).with_context(|| format!("parse Mach-O image: {path}"))?;
    let (mut uuid, mut crypt_id, mut linked_dylibs) = (None, None, Vec::new());
    let mut entitlements = BTreeMap::new();

    macro_rules! inspect_load_commands {
        ($macho:expr) => {{
            let endian = $macho.endian();
            let mut commands = $macho
                .macho_load_commands()
                .context("read Mach-O load commands")?;
            while let Some(command) = commands.next().context("read Mach-O load command")? {
                match command.variant().context("parse Mach-O load command")? {
                    LoadCommandVariant::Uuid(value) => {
                        uuid = Some(format_uuid(&value.uuid));
                    }
                    LoadCommandVariant::EncryptionInfo32(value) => {
                        crypt_id = Some(value.cryptid.get(endian));
                    }
                    LoadCommandVariant::EncryptionInfo64(value) => {
                        crypt_id = Some(value.cryptid.get(endian));
                    }
                    LoadCommandVariant::Dylib(value) => {
                        let name = command
                            .string(endian, value.dylib.name)
                            .context("read Mach-O dylib name")?;
                        linked_dylibs.push(String::from_utf8_lossy(name).into_owned());
                    }
                    LoadCommandVariant::LinkeditData(value)
                        if command.cmd() == object::macho::LC_CODE_SIGNATURE =>
                    {
                        let start = value.dataoff.get(endian) as usize;
                        let size = value.datasize.get(endian) as usize;
                        let end = start
                            .checked_add(size)
                            .context("Mach-O code-signature offset overflow")?;
                        let blob = bytes
                            .get(start..end)
                            .context("Mach-O code-signature range is out of bounds")?;
                        entitlements = codesign::entitlements_from_superblob(blob)
                            .context("parse Mach-O code-signature entitlements")?;
                    }
                    _ => {}
                }
            }
        }};
    }

    match &file {
        object::File::MachO32(macho) => inspect_load_commands!(macho),
        object::File::MachO64(macho) => inspect_load_commands!(macho),
        _ => bail!("input is not a Mach-O image: {path}"),
    }

    let mut sections = Vec::new();
    for section in file.sections() {
        let name =
            String::from_utf8_lossy(section.name_bytes().context("read Mach-O section name")?);
        let segment = section
            .segment_name_bytes()
            .context("read Mach-O segment name")?
            .unwrap_or_default();
        let segment = String::from_utf8_lossy(segment);
        let value = if segment.is_empty() {
            name.into_owned()
        } else {
            format!("{segment}/{name}")
        };
        sections.push(redact::text(&value).0);
    }

    let symbol_count = file
        .symbols()
        .filter_map(|symbol| symbol.name().ok())
        .filter(|name| !name.is_empty())
        .count();
    let mut exported_symbols = file
        .symbols()
        .filter(|symbol| symbol.scope() == SymbolScope::Dynamic && !symbol.is_undefined())
        .filter_map(|symbol| symbol.name().ok())
        .filter(|name| !name.is_empty())
        .map(|name| redact::all(name).0)
        .collect::<Vec<_>>();
    exported_symbols.sort();
    exported_symbols.dedup();

    linked_dylibs = linked_dylibs
        .into_iter()
        .map(|name| redact::all(&name).0)
        .collect();
    linked_dylibs.sort();
    linked_dylibs.dedup();
    sections.sort();
    sections.dedup();
    let objc = objc::inspect(bytes).context("parse Objective-C metadata")?;

    Ok(MachOInfo {
        path: redact::all(path).0,
        sha256: sha256(bytes),
        architecture: architecture_name(file.architecture()).into(),
        is_64: file.is_64(),
        little_endian: file.is_little_endian(),
        uuid,
        crypt_id,
        linked_dylibs,
        sections,
        symbol_count,
        exported_symbols,
        objc_classes: objc.classes,
        objc_methods: objc.methods,
        route_candidates: objc.route_candidates,
        entitlements,
    })
}

fn architecture_name(architecture: Architecture) -> &'static str {
    match architecture {
        Architecture::Aarch64 => "aarch64",
        Architecture::Aarch64_Ilp32 => "aarch64-ilp32",
        Architecture::Arm => "arm",
        Architecture::I386 => "i386",
        Architecture::X86_64 => "x86_64",
        Architecture::PowerPc => "powerpc",
        Architecture::PowerPc64 => "powerpc64",
        Architecture::Unknown => "unknown",
        _ => "unsupported",
    }
}

fn inspect_single_fat_arch<Fat: FatArch>(
    path: &str,
    bytes: &[u8],
    arches: &[Fat],
) -> Result<MachOInfo> {
    if arches.len() != 1 {
        bail!(
            "fat Mach-O must contain exactly one architecture for inventory: {path} has {}",
            arches.len()
        );
    }
    let slice = arches[0]
        .data(bytes)
        .context("read architecture slice from fat Mach-O")?;
    inspect_thin(path, slice)
}

fn format_uuid(bytes: &[u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
