// Placeholder entry point for the headless CLI tool. Prints a line that
// references `core::formula::placeholder()` so the `cli -> core` dependency
// wiring is exercised, and so `crates/cli/tests/smoke.rs` has stable output
// to assert against.
fn main() {
    println!(
        "yoko2d-cli placeholder — core: {}",
        core::formula::placeholder()
    );
}
