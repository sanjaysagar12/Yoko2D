fn main() -> eframe::Result<()> {
    app::run() // delegate to the library crate, so the actual app logic stays testable outside this thin binary wrapper
}
