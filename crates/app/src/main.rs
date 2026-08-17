struct Yoko2DApp;

impl eframe::App for Yoko2DApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Hello Yoko2D — Phase 0 scaffold");
        });
    }
}

fn main() -> eframe::Result<()> {
    // Referenced here so the `core`/`io`/`render` dependency wiring is exercised
    // even though Phase 0 has no real pattern data flowing through the app yet.
    let _pattern = core::PatternData::default();
    let _ = io::placeholder(&_pattern);
    let _ = render::placeholder(&_pattern);

    let options = eframe::NativeOptions::default();
    eframe::run_native("Yoko2D", options, Box::new(|_cc| Ok(Box::new(Yoko2DApp))))
}
