mod effects;
mod game;
mod judge;
mod models;
mod repository;
mod tja_parser;

use game::TaikoSimApp;
use models::AppConfig;

fn load_config() -> AppConfig {
    std::fs::read_to_string("config.json")
        .ok()
        .and_then(|raw| serde_json::from_str::<AppConfig>(&raw).ok())
        .unwrap_or_default()
}

fn window_conf() -> macroquad::prelude::Conf {
    let cfg = load_config();
    let mut conf = macroquad::prelude::Conf {
        window_title: "taikosim".to_owned(),
        window_width: 1280,
        window_height: 720,
        window_resizable: true,
        ..Default::default()
    };
    conf.platform.swap_interval = Some(if cfg.vsync { 1 } else { 0 });
    conf
}

#[macroquad::main(window_conf)]
async fn main() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let config = load_config();
    let mut app = TaikoSimApp::new(root, config);
    app.run().await;
}
