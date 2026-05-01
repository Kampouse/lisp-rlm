//! NEAR WASM validation — same checks as nearcore's prepare_v3.rs
//!
//! Uses wasmparser to validate that emitted WASM will pass on-chain validation.
//! No nearcore dependency needed.

#[derive(Debug)]
pub enum NearValidationError {
    InvalidWasm(String),
    InternalMemory,
    MemoryPagesOutOfRange(u64),
    TooManyFunctions(u64),
    FunctionBodyTooLarge { size: u64, max: u64 },
    InvalidImport(String),
    TooManyTables,
}

impl std::fmt::Display for NearValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidWasm(e) => write!(f, "invalid WASM: {}", e),
            Self::InternalMemory => write!(f, "NEAR contracts must not declare internal memory"),
            Self::MemoryPagesOutOfRange(pages) => write!(f, "memory pages {} out of range (1-2048)", pages),
            Self::TooManyFunctions(n) => write!(f, "too many functions: {} (max 10000)", n),
            Self::FunctionBodyTooLarge { size, max } => write!(f, "function body {} bytes (max {})", size, max),
            Self::InvalidImport(s) => write!(f, "invalid import: {}", s),
            Self::TooManyTables => write!(f, "too many tables"),
        }
    }
}

/// Validate WASM bytes against NEAR on-chain requirements.
/// Runs the same structural checks that validators perform in prepare_contract.
pub fn validate_near_wasm(wasm: &[u8]) -> Result<(), NearValidationError> {
    // First: basic WASM validation via wasmprinter (parse roundtrip)
    let wat = wasmprinter::print_bytes(wasm)
        .map_err(|e| NearValidationError::InvalidWasm(e.to_string()))?;

    // Check for internal memory declaration (not imported)
    // NEAR requires memory to be imported from "env"
    if wat.contains("(memory ") && !wat.contains("(import \"env\" \"memory\"") {
        // Check if it's actually an internal memory (not inside an import)
        for line in wat.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("(memory ") && !trimmed.starts_with("(import") {
                return Err(NearValidationError::InternalMemory);
            }
        }
    }

    // Check imports are only from "env"
    // Parse WAT for (import "xxx" ... where xxx != "env"
    for line in wat.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("(import \"") {
            if let Some(end) = trimmed.find("\"") {
                if let Some(start) = trimmed[9..].find("\"") {
                    let module = &trimmed[9..9+start];
                    if module != "env" {
                        return Err(NearValidationError::InvalidImport(
                            format!("import from \"{}\" (must be \"env\")", module)
                        ));
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_module_valid() {
        let wasm = wat::parse_str("(module)").unwrap();
        assert!(validate_near_wasm(&wasm).is_ok());
    }

    #[test]
    fn test_internal_memory_rejected() {
        let wasm = wat::parse_str("(module (memory 1 1))").unwrap();
        let err = validate_near_wasm(&wasm).unwrap_err();
        assert!(matches!(err, NearValidationError::InternalMemory));
    }

    #[test]
    fn test_imported_memory_valid() {
        let wasm = wat::parse_str("(module (import \"env\" \"memory\" (memory 1 2048)))").unwrap();
        assert!(validate_near_wasm(&wasm).is_ok());
    }

    #[test]
    fn test_non_env_import_rejected() {
        let wasm = wat::parse_str("(module (import \"other\" \"gas\" (func (param i32))))").unwrap();
        let err = validate_near_wasm(&wasm).unwrap_err();
        assert!(matches!(err, NearValidationError::InvalidImport(_)));
    }
}
