mod game;
mod models;
mod repository;
mod tja_parser;

use game::TaikoSimApp;
use serde::Deserialize;

#[derive(Deserialize, Default)]
struct BootstrapConfig {
    vsync: Option<bool>,
}

fn window_conf() -> macroquad::prelude::Conf {
    let cfg = std::fs::read_to_string("config.json")
        .ok()
        .and_then(|raw| serde_json::from_str::<BootstrapConfig>(&raw).ok())
        .unwrap_or_default();
    let vsync = cfg.vsync.unwrap_or(true);

    let mut conf = macroquad::prelude::Conf {
        window_title: "taikosim".to_owned(),
        window_width: 1280,
        window_height: 720,
        window_resizable: true,
        ..Default::default()
    };
    conf.platform.swap_interval = Some(if vsync { 1 } else { 0 });
    conf
}

#[macroquad::main(window_conf)]
async fn main() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut app = TaikoSimApp::new(root);
    app.run().await;
}
