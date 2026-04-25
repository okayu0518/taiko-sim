use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::models::SongEntry;

#[derive(Deserialize)]
struct IndexJson {
    songs: Vec<IndexSong>,
}

#[derive(Deserialize)]
struct IndexSong {
    id: String,
    title: Option<String>,
    tja_path: String,
    audio_path: Option<String>,
}

pub fn load_song_entries(data_dir: &Path) -> Vec<SongEntry> {
    let mut songs = load_from_index(&data_dir.join("index.json"), data_dir);
    if songs.is_empty() {
        songs = scan_local_song_dirs(&data_dir.join("songs"));
    }
    songs.sort_by(|a, b| a.title.cmp(&b.title));
    songs
}

fn load_from_index(index_path: &Path, data_dir: &Path) -> Vec<SongEntry> {
    let Ok(raw) = fs::read_to_string(index_path) else {
        return Vec::new();
    };
    let Ok(index) = serde_json::from_str::<IndexJson>(&raw) else {
        return Vec::new();
    };

    let mut result = Vec::new();
    for song in index.songs {
        let song_id = song.id.trim().to_string();
        if song_id.is_empty() {
            continue;
        }

        let title = song
            .title
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(&song_id)
            .to_string();

        let tja_path = data_dir.join(song.tja_path).canonicalize().unwrap_or_else(|_| PathBuf::new());
        if tja_path.as_os_str().is_empty() || !tja_path.exists() {
            continue;
        }

        let audio_path = song
            .audio_path
            .map(|rel| data_dir.join(rel))
            .and_then(|p| p.canonicalize().ok())
            .filter(|p| p.exists());

        result.push(SongEntry {
            song_id,
            title,
            tja_path,
            audio_path,
        });
    }

    result
}

fn scan_local_song_dirs(songs_dir: &Path) -> Vec<SongEntry> {
    let Ok(entries) = fs::read_dir(songs_dir) else {
        return Vec::new();
    };

    let mut result = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let Some(tja_path) = first_with_ext(&path, "tja") else {
            continue;
        };
        let audio_path = first_audio(&path);

        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        result.push(SongEntry {
            song_id: name.clone(),
            title: name,
            tja_path,
            audio_path,
        });
    }

    result
}

fn first_with_ext(dir: &Path, ext: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case(ext))
            .unwrap_or(false)
        {
            return Some(path);
        }
    }
    None
}

fn first_audio(dir: &Path) -> Option<PathBuf> {
    for ext in ["ogg", "wav", "mp3"] {
        if let Some(path) = first_with_ext(dir, ext) {
            return Some(path);
        }
    }
    None
}
