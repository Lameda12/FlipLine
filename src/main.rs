#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use flipline::app::FlipLineApp;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("FlipLine")
            .with_icon(flipline::sample::app_icon())
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([900.0, 560.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "FlipLine",
        options,
        Box::new(|cc| Ok(Box::new(FlipLineApp::new(cc)))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window().and_then(|w| w.document());
        let canvas = document
            .as_ref()
            .and_then(|d| d.get_element_by_id("flipline_canvas"))
            .and_then(|c| c.dyn_into::<web_sys::HtmlCanvasElement>().ok());
        let (Some(document), Some(canvas)) = (document, canvas) else {
            log::error!("FlipLine needs a <canvas id=\"flipline_canvas\"> in the page");
            return;
        };
        flipline::platform::web::install_pen_listeners(&canvas);
        flipline::platform::web::install_key_guards(&canvas);
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(FlipLineApp::new(cc)))),
            )
            .await;
        if let Some(loading) = document.get_element_by_id("loading") {
            match result {
                Ok(_) => loading.remove(),
                Err(e) => loading.set_inner_html(&format!("<p>FlipLine failed to start: {e:?}</p>")),
            }
        }
    });
}
