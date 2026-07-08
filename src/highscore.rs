use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const MAX_ENTRIES: usize = 10;

#[derive(Serialize, Deserialize, Clone)]
pub struct HighScoreEntry {
    pub name: String,
    pub score: u16,
    pub color: String,
    pub speed: f64,
    pub date: String,
}

fn scores_dir() -> PathBuf {
    let mut dir = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    dir.push("snake-tui");
    dir
}

fn scores_path() -> PathBuf {
    scores_dir().join("highscores.json")
}

pub fn load_scores() -> Vec<HighScoreEntry> {
    fs::read_to_string(scores_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_scores(scores: &[HighScoreEntry]) {
    let dir = scores_dir();
    if fs::create_dir_all(&dir).is_ok() {
        if let Ok(json) = serde_json::to_string_pretty(scores) {
            let _ = fs::write(scores_path(), json);
        }
    }
}

/// Whether `score` would earn a spot on the board: true if there's room left,
/// or the score beats the current lowest entry.
pub fn qualifies(score: u16) -> bool {
    let scores = load_scores();
    scores.len() < MAX_ENTRIES || scores.iter().map(|s| s.score).min().is_some_and(|min| score > min)
}

pub fn add_score(entry: HighScoreEntry) -> Vec<HighScoreEntry> {
    let mut scores = load_scores();
    scores.push(entry);
    scores.sort_by(|a, b| b.score.cmp(&a.score));
    scores.truncate(MAX_ENTRIES);
    save_scores(&scores);
    scores
}
