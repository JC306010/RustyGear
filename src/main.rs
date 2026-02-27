fn main() -> eframe::Result<()> {
    let icon = load_icon();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([160.0 * 3.0 + 40.0, 144.0 * 3.0 + 60.0])
            .with_min_inner_size([200.0, 180.0])
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "RustyGear",
        options,
        Box::new(|cc| Ok(Box::new(rustygear::gui::app::RustyGearApp::new(cc)))),
    )
}

fn load_icon() -> eframe::egui::IconData {
    let png_bytes = include_bytes!("../resources/icon.png");
    let img = image::load_from_memory(png_bytes)
        .expect("Failed to decode icon.png")
        .into_rgba8();
    let (w, h) = img.dimensions();
    eframe::egui::IconData {
        rgba: img.into_raw(),
        width: w,
        height: h,
    }
}
