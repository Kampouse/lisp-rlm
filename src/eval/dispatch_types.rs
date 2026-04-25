//! Runtime type system — predicates, contracts, and schemas.
//!
//! Three layers:
//! 1. **check / type-of / matches?** — Predicate-style runtime type assertions
//! 2. **contract** — Function-level pre/post condition checking
//! 3. **defschema / validate** — Named schemas for data shape validation
//!
//! Type language:
//! - Primitives: :nil, :bool, :int, :float, :num, :str, :sym, :list, :map, :fn, :any
//! - Parameterized: (:list :int), (:map :str :int), (:tuple :int :str)
//! - Union: (:or :int :str)
//! - Custom predicate: any function value (checked by calling it with the value)

use crate::types::LispVal;
use std::collections::HashMap as StdHashMap;

// ─────────────────────────────────────────────────────
// Type representation
// ─────────────────────────────────────────────────────

/// A runtime type descriptor. Not a LispVal — used internally.
#[derive(Clone, Debug)]
pub enum RlType {
    // Primitives
    Nil,
    Bool,
    Int,
    Float,
    Num,     // :num = int or float
    Str,
    Sym,
    List,
    Map,
    Fn,
    Any,
    // Parameterized
    ListOf(Box<RlType>),                    // (:list :int)
    MapOf(Box<RlType>, Box<RlType>),        // (:map :str :int)
    Tuple(Vec<RlType>),                      // (:tuple :int :str :bool)
    // Union
    Or(Vec<RlType>),                         // (:or :int :str)
    // Custom predicate — name of a function to call
    Predicate(String),
}

/// Parse a type descriptor from a LispVal.
/// Returns the RlType or an error string.
pub fn parse_type(t: &LispVal) -> Result<RlType, String> {
    match t {
        LispVal::Sym(s) | LispVal::Str(s) => match s.as_str() {
            ":nil" | "nil" => Ok(RlType::Nil),
            ":bool" | "bool" => Ok(RlType::Bool),
            ":int" | "int" => Ok(RlType::Int),
            ":float" | "float" => Ok(RlType::Float),
            ":num" | "number" => Ok(RlType::Num),
            ":str" | "string" => Ok(RlType::Str),
            ":sym" | "symbol" => Ok(RlType::Sym),
            ":list" => Ok(RlType::List),
            ":map" => Ok(RlType::Map),
            ":fn" | ":lambda" | "fn" | "lambda" => Ok(RlType::Fn),
            ":any" | "any" => Ok(RlType::Any),
            // Non-keyword symbol → custom predicate
            other => Ok(RlType::Predicate(other.to_string())),
        },
        LispVal::List(elems) if !elems.is_empty() => {
            let head = match &elems[0] {
                LispVal::Sym(s) | LispVal::Str(s) => s.as_str(),
                other => return Err(format!("type: expected symbol head, got {}", other)),
            };
            match head {
                ":list" | "list" => {
                    if elems.len() != 2 {
                        return Err(format!("(:list T) expects 1 type arg, got {}", elems.len() - 1));
                    }
                    Ok(RlType::ListOf(Box::new(parse_type(&elems[1])?)))
                }
                ":map" | "map" => {
                    if elems.len() != 3 {
                        return Err(format!("(:map K V) expects 2 type args, got {}", elems.len() - 1));
                    }
                    Ok(RlType::MapOf(
                        Box::new(parse_type(&elems[1])?),
                        Box::new(parse_type(&elems[2])?),
                    ))
                }
                ":tuple" | "tuple" => {
                    let inner: Result<Vec<RlType>, String> =
                        elems[1..].iter().map(parse_type).collect();
                    Ok(RlType::Tuple(inner?))
                }
                ":or" | "or" => {
                    let inner: Result<Vec<RlType>, String> =
                        elems[1..].iter().map(parse_type).collect();
                    if inner.as_ref().map(|v| v.is_empty()).unwrap_or(false) {
                        return Err("(:or ...) needs at least 1 type".into());
                    }
                    Ok(RlType::Or(inner?))
                }
                other => Err(format!("unknown compound type: {}", other)),
            }
        }
        other => Err(format!("type: expected symbol or list, got {}", other)),
    }
}

/// Format a type descriptor back to a readable string.
pub fn format_type(t: &RlType) -> String {
    match t {
        RlType::Nil => ":nil".into(),
        RlType::Bool => ":bool".into(),
        RlType::Int => ":int".into(),
        RlType::Float => ":float".into(),
        RlType::Num => ":num".into(),
        RlType::Str => ":str".into(),
        RlType::Sym => ":sym".into(),
        RlType::List => ":list".into(),
        RlType::Map => ":map".into(),
        RlType::Fn => ":fn".into(),
        RlType::Any => ":any".into(),
        RlType::ListOf(inner) => format!("(:list {})", format_type(inner)),
        RlType::MapOf(k, v) => format!("(:map {} {})", format_type(k), format_type(v)),
        RlType::Tuple(elems) => {
            let parts: Vec<String> = elems.iter().map(format_type).collect();
            format!("(:tuple {})", parts.join(" "))
        }
        RlType::Or(elems) => {
            let parts: Vec<String> = elems.iter().map(format_type).collect();
            format!("(:or {})", parts.join(" "))
        }
        RlType::Predicate(name) => name.clone(),
    }
}

/// Check if a value matches a type (without custom predicates — those need eval).
pub fn type_matches(value: &LispVal, t: &RlType) -> bool {
    match t {
        RlType::Any => true,
        RlType::Nil => matches!(value, LispVal::Nil),
        RlType::Bool => matches!(value, LispVal::Bool(_)),
        RlType::Int => matches!(value, LispVal::Num(_)),
        RlType::Float => matches!(value, LispVal::Float(_)),
        RlType::Num => matches!(value, LispVal::Num(_) | LispVal::Float(_)),
        RlType::Str => matches!(value, LispVal::Str(_)),
        RlType::Sym => matches!(value, LispVal::Sym(_)),
        RlType::List => matches!(value, LispVal::List(_)),
        RlType::Map => matches!(value, LispVal::Map(_)),
        RlType::Fn => matches!(value, LispVal::Lambda { .. }),
        RlType::ListOf(inner) => match value {
            LispVal::List(elems) => elems.iter().all(|e| type_matches(e, inner)),
            _ => false,
        },
        RlType::MapOf(kt, vt) => match value {
            LispVal::Map(m) => m.iter().all(|(k, v)| {
                type_matches(&LispVal::Str(k.clone()), kt) && type_matches(v, vt)
            }),
            _ => false,
        },
        RlType::Tuple(types) => match value {
            LispVal::List(elems) if elems.len() == types.len() => {
                elems.iter().zip(types.iter()).all(|(e, t)| type_matches(e, t))
            }
            _ => false,
        },
        RlType::Or(types) => types.iter().any(|t| type_matches(value, t)),
        RlType::Predicate(_) => {
            // Can't check without eval — return true, will be checked at runtime
            true
        }
    }
}

/// Get the type name of a value (for type-of).
pub fn type_of(value: &LispVal) -> &'static str {
    match value {
        LispVal::Nil => ":nil",
        LispVal::Bool(_) => ":bool",
        LispVal::Num(_) => ":int",
        LispVal::Float(_) => ":float",
        LispVal::Str(_) => ":str",
        LispVal::Sym(_) => ":sym",
        LispVal::List(_) => ":list",
        LispVal::Map(_) => ":map",
        LispVal::Lambda { .. } => ":fn",
        LispVal::Macro { .. } => ":macro",
        LispVal::Recur(_) => ":recur",
    }
}

// ─────────────────────────────────────────────────────
// Schema registry (global, lazy-init)
// ─────────────────────────────────────────────────────

use std::sync::LazyLock;

static SCHEMAS: LazyLock<std::sync::Mutex<StdHashMap<String, RlSchema>>> =
    LazyLock::new(|| std::sync::Mutex::new(StdHashMap::new()));

/// A named schema — a map of field names to type descriptors.
#[derive(Clone, Debug)]
pub struct RlSchema {
    pub name: String,
    pub fields: Vec<(String, RlType)>,
    pub strict: bool, // if true, reject keys not in schema
}

// ─────────────────────────────────────────────────────
// Builtin handler
// ─────────────────────────────────────────────────────

pub fn handle(name: &str, args: &[LispVal]) -> Result<Option<LispVal>, String> {
    match name {
        // ── Layer 1: Predicates ──

        "type-of" => {
            let val = args.first().ok_or("type-of: need 1 argument")?;
            Ok(Some(LispVal::Sym(type_of(val).to_string())))
        }

        "check" => {
            let val = args.first().ok_or("check: need at least 1 argument")?;
            let type_desc = args.get(1).ok_or("check: need type descriptor")?;
            let t = parse_type(type_desc)?;

            if type_matches(val, &t) {
                // Check custom predicate if present
                if let RlType::Predicate(fn_name) = &t {
                    // Custom predicates need eval — return a marker
                    // The caller should use check-pred for custom predicates
                    return Ok(Some(val.clone()));
                }
                Ok(Some(val.clone()))
            } else {
                Err(format!(
                    "check: expected {}, got {} (value: {})",
                    format_type(&t),
                    type_of(val),
                    truncate_lispval(val, 80)
                ))
            }
        }

        "check!" => {
            // Strict check — like check but errors on failure
            let val = args.first().ok_or("check!: need value")?;
            let type_desc = args.get(1).ok_or("check!: need type descriptor")?;
            let t = parse_type(type_desc)?;

            if type_matches(val, &t) {
                Ok(Some(val.clone()))
            } else {
                Err(format!(
                    "Type error: expected {}, got {} — {}",
                    format_type(&t),
                    type_of(val),
                    truncate_lispval(val, 100)
                ))
            }
        }

        "matches?" => {
            let val = args.first().ok_or("matches?: need value")?;
            let type_desc = args.get(1).ok_or("matches?: need type descriptor")?;
            let t = parse_type(type_desc)?;
            Ok(Some(LispVal::Bool(type_matches(val, &t))))
        }

        "valid-type?" => {
            // Check if a type descriptor is well-formed
            let type_desc = args.first().ok_or("valid-type?: need type descriptor")?;
            match parse_type(type_desc) {
                Ok(t) => Ok(Some(LispVal::Str(format_type(&t)))),
                Err(e) => Ok(Some(LispVal::Bool(false))),
            }
        }

        // ── Layer 3: Schemas ──

        "defschema" => {
            let name_val = args.first().ok_or("defschema: need schema name")?;
            let schema_name = match name_val {
                LispVal::Sym(s) => s.clone(),
                LispVal::Str(s) => s.clone(),
                other => return Err(format!("defschema: name must be symbol or string, got {}", other)),
            };

            // Remaining args are alternating key-type pairs
            // Optionally the last arg can be :strict
            let mut fields = Vec::new();
            let mut strict = false;
            let mut i = 1;
            while i + 1 < args.len() {
                let key = match &args[i] {
                    LispVal::Sym(s) => s.clone(),
                    LispVal::Str(s) => s.clone(),
                    other => return Err(format!("defschema: field name must be symbol or string, got {}", other)),
                };

                // Check for :strict flag
                if let LispVal::Sym(s) = &args[i + 1] {
                    if s == ":strict" {
                        strict = true;
                        break;
                    }
                }

                let t = parse_type(&args[i + 1])?;
                fields.push((key, t));
                i += 2;
            }

            // Check for :strict after all fields
            if let Some(LispVal::Sym(s)) = args.last() {
                if s == ":strict" {
                    strict = true;
                }
            }

            let schema = RlSchema {
                name: schema_name.clone(),
                fields,
                strict,
            };

            let mut schemas = SCHEMAS.lock().map_err(|e| format!("defschema: lock error: {}", e))?;
            schemas.insert(schema_name.clone(), schema);

            Ok(Some(LispVal::Str(format!("schema:{}", schema_name))))
        }

        "validate" => {
            let val = args.first().ok_or("validate: need value")?;
            let schema_name = args.get(1).ok_or("validate: need schema name")?;
            let name = match schema_name {
                LispVal::Sym(s) => s.clone(),
                LispVal::Str(s) => s.clone(),
                other => return Err(format!("validate: schema name must be symbol or string, got {}", other)),
            };

            let schemas = SCHEMAS.lock().map_err(|e| format!("validate: lock error: {}", e))?;
            let schema = schemas.get(&name)
                .ok_or_else(|| format!("validate: unknown schema '{}'", name))?;

            validate_schema(val, schema)
        }

        "schema" => {
            // Inspect a schema definition
            let schema_name = args.first().ok_or("schema: need schema name")?;
            let name = match schema_name {
                LispVal::Sym(s) => s.clone(),
                LispVal::Str(s) => s.clone(),
                other => return Err(format!("schema: name must be symbol or string, got {}", other)),
            };

            let schemas = SCHEMAS.lock().map_err(|e| format!("schema: lock error: {}", e))?;
            let schema = schemas.get(&name)
                .ok_or_else(|| format!("schema: unknown schema '{}'", name))?;

            let fields: Vec<LispVal> = schema.fields.iter()
                .map(|(k, t)| LispVal::List(vec![
                    LispVal::Str(k.clone()),
                    LispVal::Str(format_type(t)),
                ]))
                .collect();

            Ok(Some(LispVal::List(vec![
                LispVal::Str(schema.name.clone()),
                LispVal::List(fields),
                LispVal::Bool(schema.strict),
            ])))
        }

        _ => Ok(None),
    }
}

/// Validate a value against a schema.
fn validate_schema(val: &LispVal, schema: &RlSchema) -> Result<Option<LispVal>, String> {
    let map = match val {
        LispVal::Map(m) => m,
        other => return Err(format!(
            "validate: expected map for schema '{}', got {}",
            schema.name, type_of(other)
        )),
    };

    // Check all required fields
    for (field_name, field_type) in &schema.fields {
        match map.get(field_name) {
            Some(v) => {
                if !type_matches(v, field_type) {
                    return Err(format!(
                        "validate {}: field '{}' expected {}, got {} — {}",
                        schema.name,
                        field_name,
                        format_type(field_type),
                        type_of(v),
                        truncate_lispval(v, 60)
                    ));
                }
            }
            None => {
                return Err(format!(
                    "validate {}: missing required field '{}'",
                    schema.name, field_name
                ));
            }
        }
    }

    // Strict mode: check for extra keys
    if schema.strict {
        let schema_keys: std::collections::HashSet<&str> =
            schema.fields.iter().map(|(k, _)| k.as_str()).collect();
        for key in map.keys() {
            if !schema_keys.contains(key.as_str()) {
                return Err(format!(
                    "validate {}: unexpected field '{}' (strict mode)",
                    schema.name, key
                ));
            }
        }
    }

    Ok(Some(val.clone()))
}

/// Truncate a LispVal for error messages.
fn truncate_lispval(val: &LispVal, max_len: usize) -> String {
    let s = val.to_string();
    if s.len() <= max_len {
        s
    } else {
        format!("{}…", &s[..max_len])
    }
}
