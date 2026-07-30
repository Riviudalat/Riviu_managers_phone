use anyhow::{anyhow, bail, Context, Result};
use gimli::{DwarfSections, EndianSlice, RunTimeEndian, SectionId};
use object::{FileKind, Object, ObjectSection};

use crate::model::{DwarfFunction, DwarfInfo, DwarfRange};
use crate::redact;

pub fn inspect(path: &str, bytes: &[u8]) -> Result<DwarfInfo> {
    let kind = FileKind::parse(bytes)
        .map_err(|error| anyhow!("input is not a Mach-O DWARF image ({path}): {error}"))?;
    if !matches!(kind, FileKind::MachO32 | FileKind::MachO64) {
        bail!("input is not a thin Mach-O DWARF image: {path}");
    }
    let file = object::File::parse(bytes)
        .with_context(|| format!("parse Mach-O DWARF image: {}", redact::all(path).0))?;
    let endian = if file.is_little_endian() {
        RunTimeEndian::Little
    } else {
        RunTimeEndian::Big
    };

    let sections: DwarfSections<Vec<u8>> = DwarfSections::load(|id| {
        load_section(&file, id).with_context(|| format!("load DWARF section {}", id.name()))
    })?;
    let dwarf = sections.borrow(|section| EndianSlice::new(section.as_slice(), endian));

    let mut compile_units = 0_usize;
    let mut subprograms = 0_usize;
    let mut attribute_errors = 0_usize;
    let mut line_sequences = 0_usize;
    let mut line_rows = 0_usize;
    let mut line_files = Vec::new();
    let mut source_paths = Vec::new();
    let mut function_names = Vec::new();
    let mut functions = Vec::new();
    let mut units = dwarf.units();
    while let Some(header) = units.next().context("iterate DWARF unit headers")? {
        compile_units += 1;
        let unit = dwarf.unit(header).context("parse DWARF unit")?;

        let unit_name = unit
            .name
            .as_ref()
            .map(|value| value.to_string_lossy().into_owned());
        let comp_dir = unit
            .comp_dir
            .as_ref()
            .map(|value| value.to_string_lossy().into_owned());
        if let Some(path) = combine_source_path(comp_dir.as_deref(), unit_name.as_deref()) {
            source_paths.push(normalize_source_path(&path));
        }

        if let Some(program) = unit.line_program.clone() {
            let mut rows = program.rows();
            while let Some((header, row)) = rows.next_row().context("iterate DWARF line rows")? {
                line_rows += 1;
                if row.end_sequence() {
                    line_sequences += 1;
                }
                let Some(file) = header.file(row.file_index()) else {
                    continue;
                };
                match dwarf.attr_string(&unit, file.path_name()) {
                    Ok(value) => {
                        let value = value.to_string_lossy().into_owned();
                        let path =
                            combine_source_path(comp_dir.as_deref(), Some(&value)).unwrap_or(value);
                        line_files.push(normalize_source_path(&path));
                    }
                    Err(_) => attribute_errors += 1,
                }
            }
        }

        let mut entries = unit.entries();
        while let Some(entry) = entries.next_dfs().context("iterate DWARF entries")? {
            if entry.tag() != gimli::DW_TAG_subprogram {
                continue;
            }
            subprograms += 1;
            let mut attribute = None;
            for name in [
                gimli::DW_AT_linkage_name,
                gimli::DW_AT_MIPS_linkage_name,
                gimli::DW_AT_name,
            ] {
                if let Some(value) = entry.attr(name) {
                    attribute = Some(value);
                    break;
                }
            }
            let function_name = match attribute {
                Some(attribute) => match dwarf.attr_string(&unit, attribute.value()) {
                    Ok(value) => Some(redact::all(&value.to_string_lossy()).0),
                    Err(_) => {
                        attribute_errors += 1;
                        None
                    }
                },
                None => None,
            };

            let mut ranges = Vec::new();
            match dwarf.die_ranges(&unit, entry) {
                Ok(mut values) => loop {
                    match values.next() {
                        Ok(Some(value)) => ranges.push(DwarfRange {
                            begin: value.begin,
                            end: value.end,
                        }),
                        Ok(None) => break,
                        Err(_) => {
                            attribute_errors += 1;
                            break;
                        }
                    }
                },
                Err(_) => attribute_errors += 1,
            }
            ranges.sort();
            ranges.dedup();
            if let Some(name) = function_name {
                function_names.push(name.clone());
                functions.push(DwarfFunction { name, ranges });
            }
        }
    }

    line_files.sort();
    line_files.dedup();
    source_paths.sort();
    source_paths.dedup();
    function_names.sort();
    function_names.dedup();
    functions.sort();
    functions.dedup();

    Ok(DwarfInfo {
        path: redact::all(path).0,
        compile_units,
        subprograms,
        attribute_errors,
        line_sequences,
        line_rows,
        line_files,
        source_paths,
        function_names,
        functions,
    })
}

fn load_section(file: &object::File<'_>, id: SectionId) -> Result<Vec<u8>> {
    let elf_name = id.name();
    let macho_name = elf_name
        .strip_prefix('.')
        .map(|name| format!("__{name}"))
        .unwrap_or_else(|| elf_name.to_owned());
    let Some(section) = file
        .section_by_name(elf_name)
        .or_else(|| file.section_by_name(&macho_name))
    else {
        return Ok(Vec::new());
    };
    Ok(section
        .uncompressed_data()
        .context("decompress DWARF section")?
        .into_owned())
}

fn combine_source_path(comp_dir: Option<&str>, name: Option<&str>) -> Option<String> {
    match (comp_dir, name) {
        (_, Some(name)) if is_absolute_path(name) => Some(name.to_owned()),
        (Some(directory), Some(name)) => Some(format!(
            "{}/{}",
            directory.trim_end_matches(['/', '\\']),
            name.trim_start_matches(['/', '\\'])
        )),
        (None, Some(name)) => Some(name.to_owned()),
        (Some(directory), None) => Some(directory.to_owned()),
        (None, None) => None,
    }
}

fn is_absolute_path(path: &str) -> bool {
    path.starts_with('/')
        || path.starts_with('\\')
        || (path.as_bytes().get(1) == Some(&b':') && path.as_bytes()[0].is_ascii_alphabetic())
}

fn normalize_source_path(path: &str) -> String {
    redact::all(path).0.replace('\\', "/")
}
