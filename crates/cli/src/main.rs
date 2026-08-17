// Placeholder entry point for the headless CLI tool. Prints a line that
// exercises `core::formula::eval_formula` (Phase 2's real formula engine,
// superseding the old Phase 0 `formula::placeholder` stub) so the
// `cli -> core` dependency wiring stays proven end-to-end, and so
// `crates/cli/tests/smoke.rs` has stable output to assert against.
fn main() {
    let vars = std::collections::HashMap::new(); // no variables needed for this trivial smoke-test formula
    let result = core::formula::eval_formula("1+2*3", &vars) // exercise the real tokenize -> parse -> evaluate pipeline
        .map(|value| value.to_string()) // format the numeric result for printing
        .unwrap_or_else(|err| err.to_string()); // formatting rather than unwrapping keeps this binary panic-free
    println!("yoko2d-cli placeholder — core: {result}");
}
