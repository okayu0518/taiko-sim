mod game;
mod models;
mod repository;
mod tja_parser;

use game::TaikoSimApp;

fn window_conf() -> macroquad::prelude::Conf {
    macroquad::prelude::Conf {
        window_title: "taikosim".to_owned(),
        window_width: 1280,
        window_height: 720,
        ..Default::default()
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut app = TaikoSimApp::new(root);
    app.run().await;
}
