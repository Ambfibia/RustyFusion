//! Authored NPC speech lists and their requirements.

use super::mission_stage::Conditions;
use serde::{Deserialize, Serialize};
pub const FIELD: &str = "_ffoneNpcSpeech";
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpeechLine {
    pub id: i32,
    pub en: String,
    pub ru: String,
    pub audio: String,
    pub pause_seconds: u32,
    pub conditions: Conditions,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NpcSpeech {
    pub use_click: bool,
    pub use_background: bool,
    pub click: Vec<SpeechLine>,
    pub background: Vec<SpeechLine>,
}
impl NpcSpeech {
    pub fn from_row(row: &serde_json::Value) -> Result<Self, String> {
        let speech: Self = serde_json::from_value(
            row.get(FIELD)
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
        )
        .map_err(|e| e.to_string())?;
        if speech.click.len() > 128 || speech.background.len() > 128 {
            return Err("At most 128 lines per NPC speech list".into());
        }
        for list in [&speech.click, &speech.background] {
            let mut ids = std::collections::BTreeSet::new();
            if list.iter().any(|line| !ids.insert(line.id)) {
                return Err("Speech IDs must be distinct within each list".into());
            }
        }
        for line in speech.click.iter().chain(&speech.background) {
            if line.id <= 0
                || line.en.len() > 65535
                || line.ru.len() > 65535
                || line.pause_seconds > 86400
            {
                return Err("Invalid NPC speech ID, text or pause".into());
            }
            if !line.audio.is_empty()
                && (line.audio.starts_with('/')
                    || line.audio.contains(['\\', ':'])
                    || line
                        .audio
                        .split('/')
                        .any(|s| s.is_empty() || s == ".." || s == ".")
                    || !line.audio.rsplit('.').next().is_some_and(|ext| {
                        matches!(ext.to_ascii_lowercase().as_str(), "ogg" | "wav" | "mp3")
                    }))
            {
                return Err("Invalid NPC speech audio path".into());
            }
            line.conditions.validate()?;
        }
        Ok(speech)
    }
}
