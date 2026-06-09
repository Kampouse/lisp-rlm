use super::*;

impl WasmEmitter {
    pub(crate) fn call_borsh(
        &mut self,
        op: &str,
        a: &[LispVal],
    ) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "borsh-serialize" => {
                // (borsh-serialize "SchemaName" field1 field2 ...)
                if a.len() < 2 {
                    return Err("borsh-serialize requires schema name and value(s)".into());
                }
                let schema_name = match &a[0] {
                    LispVal::Str(s) => s.clone(),
                    LispVal::Sym(s) => s.clone(),
                    _ => return Err("borsh-serialize: schema name must be string or symbol".into()),
                };
                self.emit_borsh_serialize(&schema_name, &a[1..])
            }
            "borsh-deserialize" => {
                // (borsh-deserialize "SchemaName" bytes-expr)
                if a.len() < 2 {
                    return Err("borsh-deserialize requires schema name and bytes expr".into());
                }
                let schema_name = match &a[0] {
                    LispVal::Str(s) => s.clone(),
                    LispVal::Sym(s) => s.clone(),
                    _ => {
                        return Err("borsh-deserialize: schema name must be string or symbol".into())
                    }
                };
                let bytes_expr = self.expr(&a[1])?;
                self.emit_borsh_deserialize(&schema_name, bytes_expr)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}
