use std::collections::HashMap; // used by both public functions below for the variable table

use crate::variable::Variable; // needed to match on each variant when flattening
use crate::PatternData; // the container `flatten_variables` reads from

pub mod error; // FormulaError and its variants
pub mod eval; // evaluate() and the built-in function dispatch
pub mod lexer; // tokenize() and Token
pub mod parser; // parse(), Expr, and BinOp

pub use error::FormulaError; // re-exported so callers can write `formula::FormulaError`
pub use eval::evaluate; // re-exported so callers can write `formula::evaluate`
pub use lexer::{tokenize, Token}; // re-exported so callers can write `formula::tokenize`/`formula::Token`
pub use parser::{parse, BinOp, Expr}; // re-exported so callers can write `formula::parse`/`formula::Expr`/`formula::BinOp`

// The single public entry point for evaluating a formula string end to end:
// tokenize -> parse -> evaluate -> validate the final result, in that
// order, propagating the first error encountered via `?`.
pub fn eval_formula(source: &str, vars: &HashMap<String, f64>) -> Result<f64, FormulaError> {
    let tokens = tokenize(source)?; // stage 1: turn the raw source text into a token stream
    let expr = parse(&tokens)?; // stage 2: turn the token stream into an abstract syntax tree
    let result = evaluate(&expr, vars)?; // stage 3: recursively compute the AST's numeric value
    if result.is_nan() || result.is_infinite() {
        // stage 4: a finite check the sub-evaluators don't perform on every intermediate step
        return Err(FormulaError::InvalidResult); // e.g. asin(2) (out of domain) slips past the other checks as NaN
    }
    Ok(result) // the formula evaluated to a valid, finite number
}

// Builds a flat `name -> f64` map out of every variable currently stored in
// `data`, suitable for passing straight into `eval_formula`. Each
// `Variable` variant carries its numeric value under a different field
// name (`value` vs. `cached_value`), so this is where that's normalized
// away into one consistent shape.
pub fn flatten_variables(data: &PatternData) -> HashMap<String, f64> {
    data.variables() // iterate every (name, Variable) pair currently stored, from Phase 1's PatternData::variables
        .map(|(name, var)| {
            let value = match var {
                // pull out the single numeric value each variant carries, regardless of which one it is
                Variable::Measurement { value } => *value, // a measurement's own value
                Variable::Custom { cached_value, .. } => *cached_value, // a custom variable's last-evaluated value
                Variable::LineLength { value } => *value,               // a line object's length
                Variable::LineAngle { value } => *value,                // a line object's angle
                Variable::CurveLength { value } => *value,              // a curve object's length
                Variable::ArcRadius { value } => *value,                // an arc object's radius
            };
            (name.clone(), value) // pair the variable's name with its extracted numeric value
        })
        .collect() // gather all pairs into the resulting HashMap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_formula_runs_the_full_pipeline() {
        assert_eq!(eval_formula("1+2*3", &HashMap::new()).unwrap(), 7.0);
    }

    #[test]
    fn eval_formula_propagates_lexer_errors() {
        assert!(eval_formula("1+@2", &HashMap::new()).is_err());
    }

    #[test]
    fn eval_formula_propagates_parser_errors() {
        assert!(eval_formula("1+", &HashMap::new()).is_err());
    }

    #[test]
    fn eval_formula_propagates_evaluator_errors() {
        assert_eq!(
            eval_formula("1/0", &HashMap::new()).unwrap_err(),
            FormulaError::DivisionByZero
        );
    }

    #[test]
    fn flatten_variables_extracts_every_kind() {
        let mut data = PatternData::default();
        data.add_variable("waist", Variable::Measurement { value: 70.0 });
        data.add_variable(
            "half_waist",
            Variable::Custom {
                formula: "waist / 2".to_string(),
                cached_value: 35.0,
            },
        );
        data.add_variable("seam_len", Variable::LineLength { value: 12.0 });

        let flat = flatten_variables(&data);
        assert_eq!(flat.get("waist"), Some(&70.0));
        assert_eq!(flat.get("half_waist"), Some(&35.0));
        assert_eq!(flat.get("seam_len"), Some(&12.0));
        assert_eq!(flat.len(), 3);
    }

    #[test]
    fn flatten_variables_feeds_directly_into_eval_formula() {
        let mut data = PatternData::default();
        data.add_variable("height_scapula", Variable::Measurement { value: 40.0 });
        let vars = flatten_variables(&data);
        assert_eq!(eval_formula("height_scapula/10", &vars).unwrap(), 4.0);
    }

    // Minimal deterministic xorshift64* PRNG so this fuzz test is
    // reproducible without adding the `rand` crate as a new dependency.
    struct XorShift64(u64);

    impl XorShift64 {
        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
    }

    // Feeds 200 pseudo-random byte strings (decoded lossily to valid UTF-8,
    // since `eval_formula` takes `&str`) through the full pipeline with an
    // empty variable table. The only thing asserted is that none of them
    // panic — `Err` results are expected and fine, since almost all of this
    // random input is not valid formula syntax.
    #[test]
    fn eval_formula_never_panics_on_random_input() {
        let mut rng = XorShift64(0xDEAD_BEEF_CAFE_F00D); // fixed seed, so failures are reproducible
        for _ in 0..200 {
            let len = (rng.next_u64() % 40) as usize; // random length from 0 to 39 bytes
            let bytes: Vec<u8> = (0..len).map(|_| (rng.next_u64() % 256) as u8).collect();
            let text = String::from_utf8_lossy(&bytes).into_owned(); // guarantee a valid &str regardless of byte content
            let _ = eval_formula(&text, &HashMap::new()); // only panicking would fail this test
        }
    }

    // Not a correctness check: prints how long 1000 formula evaluations
    // against a ~200-entry variable table take, so the number can be
    // eyeballed against a "comfortably under 10ms" expectation. Run
    // explicitly with `cargo test -p core -- --ignored` since normal test
    // runs shouldn't depend on timing.
    #[test]
    #[ignore]
    fn timing_1000_formulas_against_200_variables() {
        let mut vars = HashMap::new();
        for i in 0..200 {
            vars.insert(format!("v{i}"), i as f64);
        }
        let formulas = [
            "v1 + v2 * v3",
            "sin(v10) + cos(v20)",
            "(v1 + v2) / (v3 + 1)",
            "sqrt(v50) ^ 2",
            "min(v1, v2) + max(v3, v4)",
            "-v5 ^ 2 + v6",
            "atan(v7) - asin(v8 / 100)",
        ];

        let start = std::time::Instant::now();
        for i in 0..1000 {
            let formula = formulas[i % formulas.len()];
            let _ = eval_formula(formula, &vars);
        }
        let elapsed = start.elapsed();

        println!(
            "evaluated 1000 formulas against {} vars in {elapsed:?}",
            vars.len()
        );
    }
}
