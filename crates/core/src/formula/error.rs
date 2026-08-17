use thiserror::Error; // brings in the `Error` derive macro used below

// Every way `tokenize`/`parse`/`evaluate`/`eval_formula` can fail while
// turning a formula string into a number. `PartialEq` lets tests compare a
// returned error directly against an expected value with `assert_eq!`.
#[derive(Debug, Clone, PartialEq, Error)] // Debug: printable; Clone: cheap to copy around; Error: implements std::error::Error via thiserror
pub enum FormulaError {
    // The lexer or parser ran into a character/token it doesn't accept at
    // this spot in the input.
    #[error("unexpected token {found:?} at position {pos}")]
    // message thiserror generates for Display/std::error::Error
    UnexpectedToken {
        found: String, // text describing the offending character or token
        pos: usize, // where it occurred: a char index from the lexer, or a token index from the parser
    },

    // The token stream ran out before a complete expression could be formed
    // (e.g. the formula ends right after an operator).
    #[error("unexpected end of input")]
    // no extra data needed: there is only one way to run out of input
    UnexpectedEof,

    // A formula referenced a variable name that the caller's variable table
    // doesn't contain an entry for.
    #[error("unknown variable {0:?}")] // {0} refers to the single tuple field below
    UnknownVariable(String), // the variable name that was looked up and not found

    // A formula called a function name that isn't one of the built-ins
    // implemented in `eval.rs`.
    #[error("unknown function {0:?}")] // {0} refers to the single tuple field below
    UnknownFunction(String), // the function name that was called and not recognized

    // A built-in function was called with a different number of arguments
    // than it requires (e.g. `sin(1, 2)`, which takes exactly one).
    #[error("{function} expects {expected} argument(s), found {found}")]
    // names all three fields by name, not position
    WrongArgCount {
        function: String, // which built-in function was miscalled
        expected: usize,  // how many arguments that function requires
        found: usize,     // how many arguments were actually supplied in the call
    },

    // The right-hand side of a `/` evaluated to exactly `0.0`. Caught
    // explicitly so formulas fail loudly instead of silently producing
    // `inf`/`NaN`, which `f64` division by zero would otherwise do.
    #[error("division by zero")] // no extra data: the operands themselves aren't retained
    DivisionByZero,

    // The fully-evaluated formula result was NaN or infinite (for example
    // from a built-in like `asin` given an out-of-domain input). Checked
    // once, at the top level, after evaluation completes.
    #[error("formula evaluated to an invalid (NaN or infinite) result")]
    // no extra data: NaN carries no useful payload to report
    InvalidResult,
}
