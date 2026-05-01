//! NEAR gas estimation using finite-wasm — same analysis as nearcore's prepare step.

use finite_wasm::{Analysis, Fee, max_stack, prefix_sum_vec::PrefixSumVec};
use finite_wasm::wasmparser as wp;

/// Calibrated from on-chain benchmarks (testnet, Apr 30 2026):
///
/// Non-recursive functions (identity, double, add, factorial):
///   Static ops × 1,115 gas + 0.6 Tgas wrapper overhead = within 0.1% of on-chain
///
/// Recursive functions (fibonacci(20), 21891 calls):
///   Extra ~31M gas per recursive call
const GAS_PER_RAW_OP: u64 = 1_115;
/// Wrapper overhead per exported function call:
/// input() + register_len() + read_register() + value_return() ≈ 0.6 Tgas
const WRAPPER_OVERHEAD_GAS: u64 = 600_000_000_000; // 0.6 Tgas
/// NEAR receipt processing overhead per call ≈ 0.3 Tgas
const RECEIPT_OVERHEAD_GAS: u64 = 300_000_000_000; // 0.3 Tgas

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
    pub stack_bytes: u64,
}

impl std::fmt::Display for GasEstimate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Contract Analysis:")?;
        writeln!(f, "  Functions:  {}", self.num_functions)?;
        writeln!(f, "  Total ops:  {}", self.total_instructions)?;
        if !self.function_details.is_empty() {
            writeln!(f, "  Per function:")?;
            for (i, fd) in self.function_details.iter().enumerate() {
                writeln!(f, "    [{}] {} ops, {} locals, {}B stack",
                    i, fd.instructions, fd.locals, fd.stack_bytes)?;
            }
        }
        let total_gas = self.total_instructions as u64 * GAS_PER_RAW_OP + WRAPPER_OVERHEAD_GAS + RECEIPT_OVERHEAD_GAS;
        let tgas = total_gas as f64 / 1e12;
        writeln!(f, "  Est. gas:   ~{:.3} Tgas (static × {} gas/op + {:.1} Tgas wrapper)",
            tgas, GAS_PER_RAW_OP, WRAPPER_OVERHEAD_GAS as f64 / 1e12)?;
        Ok(())
    }
}

/// Stack size config: each value type has a known byte size
struct StackSizeConfig;

impl max_stack::SizeConfig for StackSizeConfig {
    fn size_of_value(&self, ty: wp::ValType) -> u8 {
        match ty {
            wp::ValType::I32 | wp::ValType::F32 => 4,
            wp::ValType::I64 | wp::ValType::F64 => 8,
            wp::ValType::V128 => 16,
            wp::ValType::Ref(_) => 4,
            _ => 0,
        }
    }
    fn size_of_function_activation(&self, _locals: &PrefixSumVec<wp::ValType, u32>) -> u64 {
        16
    }
}

/// Uniform gas cost model using finite-wasm's wasmparser.
/// Implements VisitOperator (required) + VisitSimdOperator (needed by finite-wasm Config trait).
/// Uniform gas cost model using finite-wasm's wasmparser.
struct UniformCost;

macro_rules! define_dispatch {
    ($(@$faction:tt $method:ident $({ $($arg:ident: $ty:ty),* })? => $visit:ident ($($ann:tt)*))*) => {
        impl<'a> wp::VisitOperator<'a> for UniformCost {
            type Output = Fee;
            $(
                #[allow(unused_variables)]
                fn $visit(&mut self $(, $($arg: $ty),*)?) -> Self::Output {
                    // finite-wasm requires end = ZERO, everything else = 1
                    Fee::constant(if stringify!($visit) == "visit_end" || stringify!($visit) == "visit_else" { 0 } else { 1 })
                }
            )*
        }
    }
}

wp::for_each_visit_operator!(define_dispatch);

// Also implement VisitSimdOperator for finite-wasm's Config trait
macro_rules! uniform_cost_simd_impl {
    ($(@$faction:tt $method:ident $({ $($arg:ident: $ty:ty),* })? => $visit:ident ($($ann:tt)*))*) => {
        impl<'a> wp::VisitSimdOperator<'a> for UniformCost {
            $(
                #[allow(unused_variables)]
                fn $visit(&mut self $(, $($arg: $ty),*)?) -> Self::Output {
                    Fee::constant(1)
                }
            )*
        }
    }
}

wp::for_each_visit_simd_operator!(uniform_cost_simd_impl);

pub fn estimate_gas(wasm: &[u8]) -> Result<GasEstimate, String> {
    let mut analysis = Analysis::new()
        .with_stack(StackSizeConfig)
        .with_gas(UniformCost);

    let outcome = analysis.analyze(wasm)
        .map_err(|e| format!("gas analysis failed: {:?}", e))?;

    let mut details = Vec::new();
    let mut total_instructions = 0;

    for i in 0..outcome.function_frame_sizes.len() {
        let frame = outcome.function_frame_sizes.get(i).copied().unwrap_or(0);
        let ops = outcome.function_operand_stack_sizes.get(i).copied().unwrap_or(0);
        let stack = frame + ops;

        // Sum gas costs for this function
        let costs: u64 = outcome.gas_costs.get(i)
            .map(|c| c.iter().map(|f| f.constant).sum::<u64>())
            .unwrap_or(0);

        total_instructions += costs as usize;
        details.push(FuncDetail {
            instructions: costs as usize,
            locals: 0, // not directly available from analysis
            stack_bytes: stack,
        });
    }

    Ok(GasEstimate {
        num_functions: outcome.function_frame_sizes.len(),
        total_instructions,
        function_details: details,
    })
}
