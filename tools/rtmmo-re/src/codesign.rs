use std::collections::BTreeMap;
use std::io::Cursor;

use anyhow::{bail, Context, Result};
use plist::{Dictionary, Value};
use serde_json::{Number, Value as JsonValue};

use crate::redact;

const CSMAGIC_EMBEDDED_SIGNATURE: u32 = 0xfade0cc0;
const CSMAGIC_EMBEDDED_ENTITLEMENTS: u32 = 0xfade7171;
const CSSLOT_ENTITLEMENTS: u32 = 5;
const MAX_PROFILE_BYTES: usize = 32 * 1024 * 1024;

pub fn entitlements_from_superblob(bytes: &[u8]) -> Result<BTreeMap<String, JsonValue>> {
    if read_be_u32(bytes, 0, "SuperBlob magic")? != CSMAGIC_EMBEDDED_SIGNATURE {
        bail!("invalid embedded code-signature SuperBlob magic");
    }
    let declared_length = read_be_u32(bytes, 4, "SuperBlob length")? as usize;
    let count = read_be_u32(bytes, 8, "SuperBlob count")? as usize;
    if declared_length < 12 || declared_length > bytes.len() {
        bail!("code-signature SuperBlob length is out of bounds");
    }
    let index_bytes = count
        .checked_mul(8)
        .and_then(|size| 12_usize.checked_add(size))
        .context("code-signature index size overflow")?;
    if index_bytes > declared_length {
        bail!("code-signature index table is out of bounds");
    }

    for index in 0..count {
        let index_offset = 12 + index * 8;
        let slot_type = read_be_u32(bytes, index_offset, "code-signature slot type")?;
        if slot_type != CSSLOT_ENTITLEMENTS {
            continue;
        }
        let blob_offset =
            read_be_u32(bytes, index_offset + 4, "code-signature slot offset")? as usize;
        if blob_offset < index_bytes {
            bail!("code-signature entitlement offset overlaps its index table");
        }
        let blob_length_offset = blob_offset
            .checked_add(4)
            .context("entitlement blob length offset overflow")?;
        let magic = read_be_u32_within(
            bytes,
            blob_offset,
            declared_length,
            "entitlement blob magic",
        )?;
        if magic != CSMAGIC_EMBEDDED_ENTITLEMENTS {
            bail!("invalid embedded entitlement blob magic");
        }
        let blob_length = read_be_u32_within(
            bytes,
            blob_length_offset,
            declared_length,
            "entitlement blob length",
        )? as usize;
        if blob_length < 8 {
            bail!("embedded entitlement blob is shorter than its header");
        }
        let blob_end = blob_offset
            .checked_add(blob_length)
            .context("embedded entitlement blob offset overflow")?;
        if blob_end > declared_length {
            bail!("embedded entitlement blob length is out of bounds");
        }
        let payload_offset = blob_offset
            .checked_add(8)
            .context("embedded entitlement payload offset overflow")?;
        let value = Value::from_reader(Cursor::new(&bytes[payload_offset..blob_end]))
            .context("parse embedded entitlement plist")?;
        let dictionary = value
            .as_dictionary()
            .context("embedded entitlement plist root is not a dictionary")?;
        return dictionary_to_json(dictionary);
    }

    Ok(BTreeMap::new())
}

pub fn profile_entitlements(bytes: &[u8]) -> Result<BTreeMap<String, JsonValue>> {
    if bytes.len() > MAX_PROFILE_BYTES {
        bail!("embedded provisioning profile exceeds size limit");
    }
    let value = embedded_plist(bytes)?;
    let profile = value
        .as_dictionary()
        .context("provisioning profile plist root is not a dictionary")?;
    let entitlements = profile
        .get("Entitlements")
        .context("provisioning profile has no Entitlements dictionary")?
        .as_dictionary()
        .context("provisioning profile Entitlements value is not a dictionary")?;
    dictionary_to_json(entitlements)
}

fn embedded_plist(bytes: &[u8]) -> Result<Value> {
    if let Some(start) = find_bytes(bytes, b"<?xml").or_else(|| find_bytes(bytes, b"<plist")) {
        let relative_end = find_bytes(&bytes[start..], b"</plist>")
            .context("embedded XML plist has no closing tag")?;
        let end = start
            .checked_add(relative_end)
            .and_then(|value| value.checked_add(b"</plist>".len()))
            .context("embedded XML plist length overflow")?;
        return Value::from_reader(Cursor::new(&bytes[start..end]))
            .context("parse embedded XML plist");
    }

    if let Some(start) = find_bytes(bytes, b"bplist00") {
        for end in ((start + 40)..=bytes.len()).rev() {
            if !plausible_binary_trailer(&bytes[start..end]) {
                continue;
            }
            if let Ok(value) = Value::from_reader(Cursor::new(&bytes[start..end])) {
                return Ok(value);
            }
        }
        bail!("embedded binary plist is malformed");
    }

    bail!("provisioning profile contains no XML or binary plist")
}

fn plausible_binary_trailer(bytes: &[u8]) -> bool {
    if bytes.len() < 40 {
        return false;
    }
    let trailer = &bytes[bytes.len() - 32..];
    trailer[..6] == [0; 6]
        && matches!(trailer[6], 1 | 2 | 4 | 8)
        && matches!(trailer[7], 1 | 2 | 4 | 8)
}

fn dictionary_to_json(dictionary: &Dictionary) -> Result<BTreeMap<String, JsonValue>> {
    let mut output = BTreeMap::new();
    for (key, value) in dictionary {
        if sensitive_key(key) {
            continue;
        }
        let key = redact::all(key).0;
        let Some(value) = value_to_json(value)? else {
            continue;
        };
        if output.insert(key, value).is_some() {
            bail!("distinct plist keys collapse after redaction");
        }
    }
    Ok(output)
}

fn value_to_json(value: &Value) -> Result<Option<JsonValue>> {
    Ok(match value {
        Value::Boolean(value) => Some(JsonValue::Bool(*value)),
        Value::Integer(value) => value
            .as_signed()
            .map(Number::from)
            .or_else(|| value.as_unsigned().map(Number::from))
            .map(JsonValue::Number),
        Value::Real(value) => Number::from_f64(*value).map(JsonValue::Number),
        Value::String(value) => Some(JsonValue::String(redact::all(value).0)),
        Value::Array(values) => {
            let mut output = Vec::new();
            for value in values {
                if let Some(value) = value_to_json(value)? {
                    output.push(value);
                }
            }
            Some(JsonValue::Array(output))
        }
        Value::Dictionary(value) => Some(JsonValue::Object(
            dictionary_to_json(value)?.into_iter().collect(),
        )),
        Value::Data(_) | Value::Date(_) | Value::Uid(_) => None,
        _ => None,
    })
}

fn sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    normalized.contains("password")
        || normalized.contains("certificate")
        || normalized.contains("provisioneddevices")
        || normalized.contains("udid")
}

fn read_be_u32(bytes: &[u8], offset: usize, label: &str) -> Result<u32> {
    read_be_u32_within(bytes, offset, bytes.len(), label)
}

fn read_be_u32_within(bytes: &[u8], offset: usize, limit: usize, label: &str) -> Result<u32> {
    let end = offset
        .checked_add(4)
        .with_context(|| format!("{label} offset overflow"))?;
    if end > limit || end > bytes.len() {
        bail!("{label} offset is out of bounds");
    }
    Ok(u32::from_be_bytes(
        bytes[offset..end].try_into().expect("four-byte slice"),
    ))
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
