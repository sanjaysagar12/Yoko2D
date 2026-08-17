use std::collections::HashMap; // the variable lookup table `evaluate` reads from

use super::error::FormulaError; // the error type evaluation can fail with
use super::parser::{BinOp, Expr}; // the AST node types being walked

// Recursively computes the numeric value of an already-parsed `Expr`,
// looking up any `Variable` nodes in `vars`. Does not perform the final
// NaN/infinite check — that happens once, at the top level, in
// `eval_formula` (in `mod.rs`), not on every intermediate sub-result here.
pub fn evaluate(expr: &Expr, vars: &HashMap<String, f64>) -> Result<f64, FormulaError> {
    match expr {
        // dispatch on which kind of AST node this is
        Expr::Number(value) => Ok(*value), // a literal evaluates to its own value, copied out of the AST
        Expr::Variable(name) => vars
            .get(name) // look up the variable by name; returns Option<&f64>
            .copied() // turn the borrowed &f64 into an owned f64 (cheap, since f64 is Copy)
            .ok_or_else(|| FormulaError::UnknownVariable(name.clone())), // a missing name is reported, not defaulted to 0
        Expr::Neg(inner) => {
            let value = evaluate(inner, vars)?; // evaluate the operand first, propagating any error
            Ok(-value) // then negate the result
        }
        Expr::BinOp { op, lhs, rhs } => {
            let lhs = evaluate(lhs, vars)?; // evaluate the left operand first
            let rhs = evaluate(rhs, vars)?; // then the right operand
            match op {
                // dispatch on which operator this node represents
                BinOp::Add => Ok(lhs + rhs), // addition
                BinOp::Sub => Ok(lhs - rhs), // subtraction
                BinOp::Mul => Ok(lhs * rhs), // multiplication
                BinOp::Div => {
                    if rhs == 0.0 {
                        // exact zero check: qmuparser/Seamly2D formulas should error, not silently produce inf/NaN
                        Err(FormulaError::DivisionByZero) // refuse the division outright
                    } else {
                        Ok(lhs / rhs) // safe to divide
                    }
                }
                BinOp::Pow => Ok(lhs.powf(rhs)), // exponentiation; `powf` supports fractional/negative exponents, unlike `powi`
            }
        }
        Expr::Call { name, args } => call_function(name, args, vars), // function calls are handled by the helper below
    }
}

// Evaluates every argument expression of a call, then dispatches to the
// named built-in function. Returns `UnknownFunction` if `name` doesn't
// match any built-in, or `WrongArgCount` if the argument count is wrong
// for the function that *was* matched.
fn call_function(
    name: &str,
    args: &[Expr],
    vars: &HashMap<String, f64>,
) -> Result<f64, FormulaError> {
    let values: Vec<f64> = args
        .iter() // over each argument expression, in call order
        .map(|arg| evaluate(arg, vars)) // evaluate it to an f64, or propagate its error
        .collect::<Result<_, _>>()?; // gather into Result<Vec<f64>, FormulaError>, stopping at the first error

    // Builds a `WrongArgCount` error for the function being dispatched here, given how many arguments it needs.
    let wrong_arg_count = |expected: usize| FormulaError::WrongArgCount {
        function: name.to_string(), // which function was miscalled
        expected,                   // how many arguments it requires
        found: values.len(),        // how many were actually supplied
    };

    match name {
        // dispatch on the function name written in the formula
        "sin" => {
            // sine; qmuparser/Seamly2D formulas express angles in degrees, not radians
            let [x] = values.as_slice() else {
                return Err(wrong_arg_count(1)); // sin takes exactly one argument
            };
            let x = *x; // copy the argument out of the borrowed slice pattern
            Ok(x.to_radians().sin()) // convert degrees -> radians (this crate's angle convention) before calling std sin
        }
        "cos" => {
            // cosine; same degrees convention as sin
            let [x] = values.as_slice() else {
                return Err(wrong_arg_count(1));
            };
            let x = *x; // copy out the argument
            Ok(x.to_radians().cos()) // degrees -> radians before std cos
        }
        "tan" => {
            // tangent; same degrees convention as sin
            let [x] = values.as_slice() else {
                return Err(wrong_arg_count(1));
            };
            let x = *x; // copy out the argument
            Ok(x.to_radians().tan()) // degrees -> radians before std tan
        }
        "asin" => {
            // arcsine; std::f64::asin returns radians, so the result is converted back to degrees
            let [x] = values.as_slice() else {
                return Err(wrong_arg_count(1));
            };
            let x = *x; // copy out the argument
            Ok(x.asin().to_degrees()) // std asin returns radians; convert to degrees
        }
        "acos" => {
            // arccosine; result converted from radians to degrees to match this crate's convention
            let [x] = values.as_slice() else {
                return Err(wrong_arg_count(1));
            };
            let x = *x; // copy out the argument
            Ok(x.acos().to_degrees()) // std acos returns radians; convert to degrees
        }
        "atan" => {
            // arctangent; result converted from radians to degrees
            let [x] = values.as_slice() else {
                return Err(wrong_arg_count(1));
            };
            let x = *x; // copy out the argument
            Ok(x.atan().to_degrees()) // std atan returns radians; convert to degrees
        }
        "sqrt" => {
            // square root; unitless, no degree/radian conversion involved
            let [x] = values.as_slice() else {
                return Err(wrong_arg_count(1));
            };
            let x = *x; // copy out the argument
            Ok(x.sqrt())
        }
        "abs" => {
            // absolute value
            let [x] = values.as_slice() else {
                return Err(wrong_arg_count(1));
            };
            let x = *x; // copy out the argument
            Ok(x.abs())
        }
        "min" => {
            // smaller of exactly two values
            let [a, b] = values.as_slice() else {
                return Err(wrong_arg_count(2)); // min takes exactly two arguments
            };
            let (a, b) = (*a, *b); // copy both operands out of the borrowed slice pattern
            Ok(a.min(b))
        }
        "max" => {
            // larger of exactly two values
            let [a, b] = values.as_slice() else {
                return Err(wrong_arg_count(2)); // max takes exactly two arguments
            };
            let (a, b) = (*a, *b); // copy both operands out of the borrowed slice pattern
            Ok(a.max(b))
        }
        "ln" => {
            // natural logarithm
            let [x] = values.as_slice() else {
                return Err(wrong_arg_count(1));
            };
            let x = *x; // copy out the argument
            Ok(x.ln())
        }
        "log10" => {
            // base-10 logarithm
            let [x] = values.as_slice() else {
                return Err(wrong_arg_count(1));
            };
            let x = *x; // copy out the argument
            Ok(x.log10())
        }
        "exp" => {
            // e raised to the given power
            let [x] = values.as_slice() else {
                return Err(wrong_arg_count(1));
            };
            let x = *x; // copy out the argument
            Ok(x.exp())
        }
        "round" => {
            // round to the nearest integer, ties away from zero (std f64::round's behavior)
            let [x] = values.as_slice() else {
                return Err(wrong_arg_count(1));
            };
            let x = *x; // copy out the argument
            Ok(x.round())
        }
        other => Err(FormulaError::UnknownFunction(other.to_string())), // no built-in matches this name
    }
}

#[cfg(test)]
mod tests {
    use super::super::lexer::tokenize;
    use super::super::parser::parse;
    use super::*;

    fn eval_str(source: &str, vars: &HashMap<String, f64>) -> Result<f64, FormulaError> {
        let tokens = tokenize(source).unwrap();
        let expr = parse(&tokens).unwrap();
        evaluate(&expr, vars)
    }

    #[test]
    fn evaluates_operator_precedence() {
        assert_eq!(eval_str("1+2*3", &HashMap::new()).unwrap(), 7.0);
    }

    #[test]
    fn evaluates_parenthesized_grouping() {
        assert_eq!(eval_str("(1+2)*3", &HashMap::new()).unwrap(), 9.0);
    }

    #[test]
    fn evaluates_right_associative_power() {
        // 2^3^2 = 2^(3^2) = 2^9 = 512, not (2^3)^2 = 64
        assert_eq!(eval_str("2^3^2", &HashMap::new()).unwrap(), 512.0);
    }

    #[test]
    fn evaluates_unary_minus_looser_than_power() {
        // -2^2 = -(2^2) = -4, not (-2)^2 = 4
        assert_eq!(eval_str("-2^2", &HashMap::new()).unwrap(), -4.0);
    }

    #[test]
    fn sin_operates_on_degrees_not_radians() {
        let result = eval_str("sin(90)", &HashMap::new()).unwrap();
        assert!((result - 1.0).abs() < 1e-9);
    }

    #[test]
    fn evaluates_variable_lookup() {
        let mut vars = HashMap::new();
        vars.insert("height_scapula".to_string(), 40.0);
        assert_eq!(eval_str("height_scapula/10", &vars).unwrap(), 4.0);
    }

    #[test]
    fn missing_variable_is_reported() {
        let err = eval_str("missing_var", &HashMap::new()).unwrap_err();
        assert_eq!(
            err,
            FormulaError::UnknownVariable("missing_var".to_string())
        );
    }

    #[test]
    fn unknown_function_is_reported() {
        let err = eval_str("bogus_fn(1)", &HashMap::new()).unwrap_err();
        assert_eq!(err, FormulaError::UnknownFunction("bogus_fn".to_string()));
    }

    #[test]
    fn wrong_arg_count_is_reported() {
        let err = eval_str("sin(1,2)", &HashMap::new()).unwrap_err();
        assert_eq!(
            err,
            FormulaError::WrongArgCount {
                function: "sin".to_string(),
                expected: 1,
                found: 2,
            }
        );
    }

    #[test]
    fn division_by_zero_is_reported() {
        let err = eval_str("1/0", &HashMap::new()).unwrap_err();
        assert_eq!(err, FormulaError::DivisionByZero);
    }
}
