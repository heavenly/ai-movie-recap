use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipPlan {
    pub start: i32,
    pub end: i32,
    pub narration: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClipPlanList {
    pub items: Vec<ClipPlan>,
}

#[derive(Debug, Deserialize)]
struct ClipPlanRoot {
    clips: Vec<ClipPlan>,
}

impl ClipPlanList {
    pub fn from_json(text: &str) -> Result<Self> {
        let root: ClipPlanRoot =
            serde_json::from_str(text).with_context(|| "Failed to parse clip plan JSON")?;
        Ok(Self { items: root.clips })
    }
}
