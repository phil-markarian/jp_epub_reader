use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceRef {
    pub kind: SourceKind,
    pub id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Aozora,
    Web,
    PlainText,
    Ocr,
    Manual,
}

impl SourceRef {
    pub fn aozora(work_id: u32) -> Self {
        Self {
            kind: SourceKind::Aozora,
            id: work_id.to_string(),
        }
    }
}
