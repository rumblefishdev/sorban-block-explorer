//! Contract interface extraction from WASM bytecode in LedgerEntryChanges.
//!
//! When a contract is deployed, `ContractCodeEntry` appears in ledger entry changes.
//! This module extracts the WASM `contractspecv0` custom section, deserializes
//! `ScSpecEntry` values, and produces `ExtractedContractInterface` with function
//! signatures for storage in `soroban_contracts.metadata`.

use stellar_xdr::curr::*;

use crate::types::{ContractFunction, ExtractedContractInterface, FunctionParam};

/// Extract contract interfaces from all `ContractCodeEntry` items found in
/// the transaction meta's ledger entry changes.
///
/// Returns one `ExtractedContractInterface` per new WASM deployment found.
/// Non-Soroban transactions and transactions without new code produce an empty vec.
pub fn extract_contract_interfaces(tx_meta: &TransactionMeta) -> Vec<ExtractedContractInterface> {
    let changes = collect_ledger_changes(tx_meta);
    let mut interfaces = Vec::new();

    for change in changes {
        let entry = match change {
            LedgerEntryChange::Created(e) | LedgerEntryChange::Updated(e) => e,
            _ => continue,
        };
        if let LedgerEntryData::ContractCode(ref code_entry) = entry.data
            && let Some(iface) = parse_contract_code(code_entry)
        {
            interfaces.push(iface);
        }
    }

    interfaces
}

/// Collect all LedgerEntryChange refs from a TransactionMeta.
fn collect_ledger_changes(meta: &TransactionMeta) -> Vec<&LedgerEntryChange> {
    let mut changes = Vec::new();

    match meta {
        TransactionMeta::V3(v3) => {
            collect_from_entry_changes(&v3.tx_changes_before, &mut changes);
            for op_meta in v3.operations.iter() {
                collect_from_entry_changes(&op_meta.changes, &mut changes);
            }
            collect_from_entry_changes(&v3.tx_changes_after, &mut changes);
        }
        TransactionMeta::V4(v4) => {
            collect_from_entry_changes(&v4.tx_changes_before, &mut changes);
            for op_meta in v4.operations.iter() {
                collect_from_entry_changes(&op_meta.changes, &mut changes);
            }
            collect_from_entry_changes(&v4.tx_changes_after, &mut changes);
        }
        _ => {}
    }

    changes
}

fn collect_from_entry_changes<'a>(
    changes: &'a LedgerEntryChanges,
    out: &mut Vec<&'a LedgerEntryChange>,
) {
    for change in changes.iter() {
        out.push(change);
    }
}

/// Parse a single ContractCodeEntry into an ExtractedContractInterface.
fn parse_contract_code(code_entry: &ContractCodeEntry) -> Option<ExtractedContractInterface> {
    let wasm_bytes = code_entry.code.as_slice();
    let wasm_hash = hex::encode(code_entry.hash.0);
    let wasm_byte_len = wasm_bytes.len();

    let spec_bytes = extract_custom_section(wasm_bytes, "contractspecv0")?;
    let functions = parse_spec_entries(&spec_bytes);

    Some(ExtractedContractInterface {
        wasm_hash,
        functions,
        wasm_byte_len,
    })
}

/// Extract a named custom section from WASM binary.
///
/// WASM format: magic (4) + version (4) + sections.
/// Each section: section_id (1 byte), size (LEB128), data.
/// Custom section (id=0): name_len (LEB128), name (UTF-8), content.
fn extract_custom_section(wasm: &[u8], section_name: &str) -> Option<Vec<u8>> {
    if wasm.len() < 8 {
        return None;
    }
    // Validate WASM magic + version
    if &wasm[0..4] != b"\x00asm" {
        return None;
    }

    let mut pos = 8; // skip magic + version

    while pos < wasm.len() {
        let section_id = wasm[pos];
        pos = pos.checked_add(1)?;

        let (section_size, bytes_read) = read_leb128(wasm.get(pos..)?)?;
        pos = pos.checked_add(bytes_read)?;
        let section_size = section_size as usize;

        if section_id == 0 {
            // Custom section — read the name
            let section_start = pos;
            let (name_len, name_bytes_read) = read_leb128(wasm.get(pos..)?)?;
            pos = pos.checked_add(name_bytes_read)?;

            let name_len = name_len as usize;
            let name_end = pos.checked_add(name_len)?;
            if name_end > wasm.len() {
                return None;
            }
            let name = std::str::from_utf8(&wasm[pos..name_end]).ok()?;
            pos = name_end;

            let header_bytes = pos.checked_sub(section_start)?;
            if name == section_name {
                let content_len = section_size.checked_sub(header_bytes)?;
                let end = pos.checked_add(content_len)?;
                if end > wasm.len() {
                    return None;
                }
                return Some(wasm[pos..end].to_vec());
            }

            // Skip rest of this custom section
            let remaining = section_size.checked_sub(header_bytes)?;
            pos = pos.checked_add(remaining)?;
        } else {
            // Skip non-custom section
            pos = pos.checked_add(section_size)?;
        }
    }

    None
}

/// Read a LEB128-encoded u32. Returns (value, bytes_consumed).
fn read_leb128(bytes: &[u8]) -> Option<(u32, usize)> {
    let mut result: u32 = 0;
    let mut shift = 0;
    for (i, &byte) in bytes.iter().enumerate() {
        if shift >= 35 {
            return None; // overflow
        }
        result |= ((byte & 0x7F) as u32) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            return Some((result, i + 1));
        }
    }
    None
}

/// Parse a stream of XDR-encoded ScSpecEntry values from the contractspecv0 section.
fn parse_spec_entries(spec_bytes: &[u8]) -> Vec<ContractFunction> {
    let mut functions = Vec::new();
    let mut pos = 0;

    while pos < spec_bytes.len() {
        let remaining = &spec_bytes[pos..];
        let mut cursor = std::io::Cursor::new(remaining);
        let limits = Limits {
            len: remaining.len(),
            depth: 512,
        };
        let mut limited = Limited::new(&mut cursor, limits);

        match ScSpecEntry::read_xdr(&mut limited) {
            Ok(entry) => {
                pos += cursor.position() as usize;
                if let ScSpecEntry::FunctionV0(func) = entry {
                    functions.push(spec_function_to_contract_function(&func));
                }
            }
            Err(_) => break,
        }
    }

    functions
}

/// Convert an ScSpecFunctionV0 into our ContractFunction type.
fn spec_function_to_contract_function(func: &ScSpecFunctionV0) -> ContractFunction {
    let name = std::str::from_utf8(func.name.as_vec())
        .unwrap_or("<invalid-utf8>")
        .to_string();

    let doc = std::str::from_utf8(func.doc.as_vec())
        .unwrap_or("")
        .to_string();

    let inputs = func
        .inputs
        .iter()
        .map(|input| FunctionParam {
            name: std::str::from_utf8(input.name.as_vec())
                .unwrap_or("<invalid-utf8>")
                .to_string(),
            type_name: spec_type_to_string(&input.type_),
        })
        .collect();

    let outputs = func.outputs.iter().map(spec_type_to_string).collect();

    ContractFunction {
        name,
        doc,
        inputs,
        outputs,
    }
}

/// Convert an ScSpecTypeDef to a human-readable type string.
fn spec_type_to_string(t: &ScSpecTypeDef) -> String {
    match t {
        ScSpecTypeDef::Val => "val".into(),
        ScSpecTypeDef::Bool => "bool".into(),
        ScSpecTypeDef::Void => "void".into(),
        ScSpecTypeDef::Error => "error".into(),
        ScSpecTypeDef::U32 => "u32".into(),
        ScSpecTypeDef::I32 => "i32".into(),
        ScSpecTypeDef::U64 => "u64".into(),
        ScSpecTypeDef::I64 => "i64".into(),
        ScSpecTypeDef::U128 => "u128".into(),
        ScSpecTypeDef::I128 => "i128".into(),
        ScSpecTypeDef::U256 => "u256".into(),
        ScSpecTypeDef::I256 => "i256".into(),
        ScSpecTypeDef::Timepoint => "timepoint".into(),
        ScSpecTypeDef::Duration => "duration".into(),
        ScSpecTypeDef::Bytes => "bytes".into(),
        ScSpecTypeDef::String => "string".into(),
        ScSpecTypeDef::Symbol => "symbol".into(),
        ScSpecTypeDef::Address => "address".into(),
        ScSpecTypeDef::Option(inner) => {
            format!("option<{}>", spec_type_to_string(&inner.value_type))
        }
        ScSpecTypeDef::Result(inner) => format!(
            "result<{}, {}>",
            spec_type_to_string(&inner.ok_type),
            spec_type_to_string(&inner.error_type)
        ),
        ScSpecTypeDef::Vec(inner) => format!("vec<{}>", spec_type_to_string(&inner.element_type)),
        ScSpecTypeDef::Map(inner) => format!(
            "map<{}, {}>",
            spec_type_to_string(&inner.key_type),
            spec_type_to_string(&inner.value_type)
        ),
        ScSpecTypeDef::Tuple(inner) => {
            let types: Vec<String> = inner.value_types.iter().map(spec_type_to_string).collect();
            format!("tuple<{}>", types.join(", "))
        }
        ScSpecTypeDef::BytesN(inner) => format!("bytes{}", inner.n),
        ScSpecTypeDef::Udt(inner) => std::str::from_utf8(inner.name.as_vec())
            .unwrap_or("<invalid-utf8>")
            .to_string(),
        ScSpecTypeDef::MuxedAddress => "muxed_address".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_leb128_single_byte() {
        assert_eq!(read_leb128(&[0x05]), Some((5, 1)));
        assert_eq!(read_leb128(&[0x7F]), Some((127, 1)));
    }

    #[test]
    fn read_leb128_multi_byte() {
        // 128 = 0x80 0x01
        assert_eq!(read_leb128(&[0x80, 0x01]), Some((128, 2)));
        // 300 = 0xAC 0x02
        assert_eq!(read_leb128(&[0xAC, 0x02]), Some((300, 2)));
    }

    #[test]
    fn extract_custom_section_from_minimal_wasm() {
        // Build a minimal WASM with a custom section named "test" containing [1,2,3]
        let mut wasm = Vec::new();
        wasm.extend_from_slice(b"\x00asm"); // magic
        wasm.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]); // version 1

        // Custom section: id=0, size=8, name_len=4, name="test", content=[1,2,3]
        wasm.push(0x00); // section id = custom
        wasm.push(0x08); // section size = 8 bytes
        wasm.push(0x04); // name length = 4
        wasm.extend_from_slice(b"test"); // name
        wasm.extend_from_slice(&[1, 2, 3]); // content

        let result = extract_custom_section(&wasm, "test");
        assert_eq!(result, Some(vec![1, 2, 3]));
    }

    #[test]
    fn extract_custom_section_not_found() {
        let mut wasm = Vec::new();
        wasm.extend_from_slice(b"\x00asm");
        wasm.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);

        // Custom section with name "other"
        wasm.push(0x00);
        wasm.push(0x09);
        wasm.push(0x05);
        wasm.extend_from_slice(b"other");
        wasm.extend_from_slice(&[1, 2, 3]);

        let result = extract_custom_section(&wasm, "contractspecv0");
        assert!(result.is_none());
    }

    #[test]
    fn extract_custom_section_skips_non_custom() {
        let mut wasm = Vec::new();
        wasm.extend_from_slice(b"\x00asm");
        wasm.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);

        // Non-custom section (type section, id=1)
        wasm.push(0x01); // section id = type
        wasm.push(0x03); // section size = 3
        wasm.extend_from_slice(&[0xAA, 0xBB, 0xCC]); // section data

        // Custom section with target name
        wasm.push(0x00); // custom
        wasm.push(0x07); // size = 7
        wasm.push(0x04); // name len = 4
        wasm.extend_from_slice(b"test");
        wasm.extend_from_slice(&[42, 43]);

        let result = extract_custom_section(&wasm, "test");
        assert_eq!(result, Some(vec![42, 43]));
    }

    #[test]
    fn invalid_wasm_returns_none() {
        assert!(extract_custom_section(&[], "test").is_none());
        assert!(extract_custom_section(b"not wasm", "test").is_none());
    }

    #[test]
    fn no_interfaces_for_non_soroban_meta() {
        let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: LedgerEntryChanges::default(),
            operations: VecM::default(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: None,
        });

        let result = extract_contract_interfaces(&tx_meta);
        assert!(result.is_empty());
    }

    #[test]
    fn spec_type_to_string_primitives() {
        assert_eq!(spec_type_to_string(&ScSpecTypeDef::Bool), "bool");
        assert_eq!(spec_type_to_string(&ScSpecTypeDef::U128), "u128");
        assert_eq!(spec_type_to_string(&ScSpecTypeDef::Address), "address");
    }

    #[test]
    fn spec_type_to_string_compound() {
        let opt = ScSpecTypeDef::Option(Box::new(ScSpecTypeOption {
            value_type: Box::new(ScSpecTypeDef::Address),
        }));
        assert_eq!(spec_type_to_string(&opt), "option<address>");

        let vec = ScSpecTypeDef::Vec(Box::new(ScSpecTypeVec {
            element_type: Box::new(ScSpecTypeDef::U64),
        }));
        assert_eq!(spec_type_to_string(&vec), "vec<u64>");

        let map = ScSpecTypeDef::Map(Box::new(ScSpecTypeMap {
            key_type: Box::new(ScSpecTypeDef::Symbol),
            value_type: Box::new(ScSpecTypeDef::I128),
        }));
        assert_eq!(spec_type_to_string(&map), "map<symbol, i128>");
    }
}
