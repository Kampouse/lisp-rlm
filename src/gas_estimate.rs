//! NEAR gas estimation by counting WASM instructions.

use wasmparser::{Parser, Payload, OperatorsReader};

const MAX_STACK_HEIGHT: u64 = 16 * 1024;

#[derive(Debug)]
pub struct GasEstimate {
    pub num_functions: usize,
    pub total_instructions: usize,
    pub function_details: Vec<FuncDetail>,
}

#[derive(Debug)]
pub struct FuncDetail {
    pub instructions: usize,
    pub locals: usize,
}

impl std::fmt::Display for GasEstimate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Contract Analysis:")?;
        writeln!(f, "  Functions:  {}", self.num_functions)?;
        writeln!(f, "  Total ops:  {}", self.total_instructions)?;
        if !self.function_details.is_empty() {
            writeln!(f, "  Per function:")?;
            for (i, fd) in self.function_details.iter().enumerate() {
                writeln!(f, "    [{}] {} ops, {} locals",
                    i, fd.instructions, fd.locals)?;
            }
        }
        // NEAR charges ~100 gas per WASM instruction (rough)
        let estimated_tgas = self.total_instructions as f64 * 100.0 / 1_000_000_000_000.0;
        writeln!(f, "  Est. gas:   ~{:.3} Tgas", estimated_tgas.max(0.001))?;
        Ok(())
    }
}

pub fn estimate_gas(wasm: &[u8]) -> Result<GasEstimate, String> {
    let parser = Parser::new(0);
    let mut func_details: Vec<FuncDetail> = Vec::new();
    let mut total_instructions = 0;

    for payload in parser.parse_all(wasm) {
        let payload = payload.map_err(|e| format!("parse error: {}", e))?;
        if let Payload::CodeSectionEntry(func_body) = payload {
            let locals_count: usize = func_body.get_locals_reader()
                .map(|r| r.into_iter().count())
                .unwrap_or(0);

            let ops_reader: OperatorsReader = func_body.get_operators_reader()
                .map_err(|e| format!("ops reader: {}", e))?;

            let count = ops_reader.into_iter()
                .count();

            total_instructions += count;
            func_details.push(FuncDetail {
                instructions: count,
                locals: locals_count,
            });
        }
    }

    Ok(GasEstimate {
        num_functions: func_details.len(),
        total_instructions,
        function_details: func_details,
    })
}
