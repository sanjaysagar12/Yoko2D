/// Stands in for the formula evaluator that will parse and compute
/// `Variable::Custom` formula strings (and derived lengths/angles) in a
/// later phase. Exists now purely so other modules/crates have something to
/// call and so the `formula` module's public shape is settled early.
pub fn placeholder() -> &'static str {
    "formula engine not yet implemented"
}
