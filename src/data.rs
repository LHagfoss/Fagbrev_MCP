use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DashboardOverview {
    pub authenticated: bool,
    pub page_title: String,
    pub url: String,
    pub progress_percent: Option<u8>,
    pub documentations_written: Option<u32>,
    pub assessment_conversations: Option<u32>,
    pub documentations_needing_correction: Option<u32>,
    pub documentations_to_review: Option<u32>,
    pub approved_documentations: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompetencyGoal {
    pub number: u8,
    pub title: String,
    pub status_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompetencyGoalDetails {
    pub number: u8,
    pub url: String,
    pub page_title: String,
    pub visible_text: String,
}
