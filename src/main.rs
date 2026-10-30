mod effects;
mod game;
mod gameplay;
mod judge;
mod models;
mod repository;
mod tja_parser;

use std::path::{Path, PathBuf};
use game::TaikoSimApp;
use models::AppConfig;

fn resource_root_from(exe_dir: &Path, cwd: Option<&Path>) -> PathBuf {
    let has_resources = |dir: &Path| {
        dir.join("assets").is_dir() || dir.join("data").is_dir() || dir.join("config.json").is_file()
    };
    if has_resources(exe_dir) {
        return exe_dir.to_path_buf();
    }
    if let Some(cwd) = cwd {
        if has_resources(cwd) {
            return cwd.to_path_buf();
        }
    }
    exe_dir.to_path_buf()
}

fn resource_root() -> PathBuf {
    let exe = std::env::current_exe().ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let cwd = std::env::current_dir().ok();
    resource_root_from(&exe, cwd.as_deref())
}

fn load_config(root: &Path) -> AppConfig {
    std::fs::read_to_string(root.join("config.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<AppConfig>(&raw).ok())
        .unwrap_or_default()
}

fn window_conf() -> macroquad::prelude::Conf {
    let cfg = load_config(&resource_root());
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
    let root = resource_root();
    let config = load_config(&root);
    let mut app = TaikoSimApp::new(root, config);
    app.run().await;
}

#[cfg(test)]
mod portable_tests {
    use super::*;
    #[test]
    fn executable_directory_is_used_when_it_contains_assets() {
        let exe = std::env::temp_dir().join(format!("taiko-exe-{}", std::process::id()));
        let cwd = std::env::temp_dir().join(format!("taiko-cwd-{}", std::process::id()));
        std::fs::create_dir_all(exe.join("assets")).unwrap();
        std::fs::create_dir_all(cwd.join("assets")).unwrap();
        assert_eq!(resource_root_from(&exe, Some(&cwd)), exe);
        std::fs::remove_dir_all(exe).unwrap();
        std::fs::remove_dir_all(cwd).unwrap();
    }
}
