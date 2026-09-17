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

        let tja_files = all_with_ext(&path, "tja");
        if tja_files.is_empty() {
            continue;
        }
        let audio_path = first_audio(&path);

        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let multi = tja_files.len() > 1;
        for tja_path in tja_files {
            let stem = tja_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("chart")
                .to_string();

            let title = if multi {
                format!("{} / {}", name, stem)
            } else {
                name.clone()
            };

            result.push(SongEntry {
                song_id: format!("{}:{}", name, stem),
                title,
                tja_path,
                audio_path: audio_path.clone(),
            });
        }
    }

    result
}

fn first_with_ext(dir: &Path, ext: &str) -> Option<PathBuf> {
    all_with_ext(dir, ext).into_iter().next()
}

fn all_with_ext(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case(ext))
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
    out.sort();
    out
}

fn first_audio(dir: &Path) -> Option<PathBuf> {
    for ext in ["ogg", "wav", "mp3"] {
        if let Some(path) = first_with_ext(dir, ext) {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Temporary data directory removed on drop.
    struct TempDataDir(PathBuf);

    impl TempDataDir {
        fn new() -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("taikosim-test-{}-{}", std::process::id(), n));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn touch(&self, rel: &str) -> PathBuf {
            self.write(rel, "")
        }

        fn write(&self, rel: &str, content: &str) -> PathBuf {
            let path = self.0.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, content).unwrap();
            path
        }
    }

    impl Drop for TempDataDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn file_name(path: &Option<PathBuf>) -> Option<String> {
        path.as_ref().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().into_owned())
    }

    #[test]
    fn missing_data_dir_yields_no_songs() {
        let tmp = TempDataDir::new();
        assert!(load_song_entries(&tmp.0.join("nope")).is_empty());
    }

    #[test]
    fn single_tja_uses_folder_name_as_title() {
        let tmp = TempDataDir::new();
        let tja = tmp.touch("songs/SongA/chart.tja");
        tmp.touch("songs/SongA/chart.ogg");

        let songs = load_song_entries(&tmp.0);
        assert_eq!(songs.len(), 1);
        assert_eq!(songs[0].title, "SongA");
        assert_eq!(songs[0].song_id, "SongA:chart");
        assert_eq!(songs[0].tja_path, tja);
        assert_eq!(file_name(&songs[0].audio_path).as_deref(), Some("chart.ogg"));
    }

    #[test]
    fn multiple_tja_in_folder_are_listed_separately() {
        let tmp = TempDataDir::new();
        tmp.touch("songs/Pack/y.tja");
        tmp.touch("songs/Pack/x.tja");
        tmp.touch("songs/Pack/song.wav");

        let songs = load_song_entries(&tmp.0);
        let titles: Vec<_> = songs.iter().map(|s| s.title.as_str()).collect();
        let ids: Vec<_> = songs.iter().map(|s| s.song_id.as_str()).collect();
        assert_eq!(titles, vec!["Pack / x", "Pack / y"]);
        assert_eq!(ids, vec!["Pack:x", "Pack:y"]);
        assert!(songs.iter().all(|s| file_name(&s.audio_path).as_deref() == Some("song.wav")));
    }

    #[test]
    fn audio_is_chosen_by_extension_priority() {
        let tmp = TempDataDir::new();
        for f in ["All/a.tja", "All/a.mp3", "All/a.wav", "All/a.ogg", "NoOgg/a.tja", "NoOgg/a.mp3", "NoOgg/a.wav", "None/a.tja"] {
            tmp.touch(&format!("songs/{f}"));
        }

        let songs = load_song_entries(&tmp.0);
        let audio: Vec<_> = songs.iter().map(|s| (s.title.as_str(), file_name(&s.audio_path))).collect();
        assert_eq!(audio, vec![
            ("All", Some("a.ogg".to_string())),
            ("NoOgg", Some("a.wav".to_string())),
            ("None", None),
        ]);
    }

    #[test]
    fn extensions_are_case_insensitive() {
        let tmp = TempDataDir::new();
        tmp.touch("songs/Upper/SONG.TJA");
        tmp.touch("songs/Upper/SONG.OGG");

        let songs = load_song_entries(&tmp.0);
        assert_eq!(songs.len(), 1);
        assert!(songs[0].audio_path.is_some());
    }

    #[test]
    fn folders_without_tja_and_loose_files_are_skipped() {
        let tmp = TempDataDir::new();
        tmp.touch("songs/AudioOnly/song.ogg");
        tmp.touch("songs/loose.tja");
        tmp.touch("songs/Valid/song.tja");

        let titles: Vec<_> = load_song_entries(&tmp.0).into_iter().map(|s| s.title).collect();
        assert_eq!(titles, vec!["Valid"]);
    }

    #[test]
    fn songs_are_sorted_by_title() {
        let tmp = TempDataDir::new();
        for name in ["c", "a", "b"] {
            tmp.touch(&format!("songs/{name}/song.tja"));
        }
        let titles: Vec<_> = load_song_entries(&tmp.0).into_iter().map(|s| s.title).collect();
        assert_eq!(titles, vec!["a", "b", "c"]);
    }

    #[test]
    fn index_json_entries_are_loaded_and_validated() {
        let tmp = TempDataDir::new();
        let tja = tmp.touch("charts/a.tja");
        let audio = tmp.touch("charts/a.ogg");
        tmp.touch("charts/b.tja");
        tmp.write("index.json", r#"{"songs": [
            {"id": "a", "title": "Song A", "tja_path": "charts/a.tja", "audio_path": "charts/a.ogg"},
            {"id": "b", "title": "  ", "tja_path": "charts/b.tja", "audio_path": "charts/missing.ogg"},
            {"id": "missing", "tja_path": "charts/missing.tja"},
            {"id": "  ", "tja_path": "charts/a.tja"}
        ]}"#);
        // Local songs are ignored while the index has valid entries.
        tmp.touch("songs/Local/song.tja");

        let songs = load_song_entries(&tmp.0);
        let summary: Vec<_> = songs.iter().map(|s| (s.song_id.as_str(), s.title.as_str(), s.audio_path.is_some())).collect();
        assert_eq!(summary, vec![("a", "Song A", true), ("b", "b", false)]);
        assert_eq!(songs[0].tja_path, tja.canonicalize().unwrap());
        assert_eq!(songs[0].audio_path, Some(audio.canonicalize().unwrap()));
    }

    #[test]
    fn invalid_or_empty_index_falls_back_to_local_scan() {
        let tmp = TempDataDir::new();
        tmp.touch("songs/Local/song.tja");

        tmp.write("index.json", "{ not json");
        assert_eq!(load_song_entries(&tmp.0).len(), 1);

        tmp.write("index.json", r#"{"songs": [{"id": "x", "tja_path": "missing.tja"}]}"#);
        assert_eq!(load_song_entries(&tmp.0)[0].title, "Local");
    }
}
