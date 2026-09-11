//! Stable read-model contracts shared by the browser adapter and MCP layer.
//!
//! These types intentionally describe what can be observed in Fagbrev.io. In
//! particular, a `Delmal` may only be a piece of activity text rendered inside
//! a competency goal. Its optional `id` must not be treated as present until
//! the site exposes an independent identifier.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The documentation states currently relevant to the dashboard workflow.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentationStatus {
    Draft,
    Submitted,
    InReview,
    NeedsCorrection,
    Approved,
    Rejected,
    #[default]
    Unknown,
}

/// A competency goal reference suitable for attaching a documentation record.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct CompetencyGoalReference {
    pub number: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Fagbrev's internal identifier, when the UI exposes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// A sub-goal/work activity belonging to a competency goal.
///
/// `id` and `number` are optional because the current plan UI appears to
/// render delmål as activity text rather than independent records. `number`,
/// when present, is the ordinal within the parent goal, not an assumed remote
/// database ID.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct Delmal {
    pub goal_number: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u16>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<DocumentationStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_count: Option<u32>,
}

/// A target can refer to a whole competency goal, a delmål, or both.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DocumentationTarget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub competency_goal: Option<CompetencyGoalReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delmal: Option<Delmal>,
}

/// A documentation item as observed in the learner's account.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DocumentationRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_html: Option<String>,
    #[serde(default)]
    pub status: DocumentationStatus,
    #[serde(default)]
    pub target: DocumentationTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<String>,
    #[serde(default)]
    pub attachments: Vec<DocumentationAttachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_summary: Option<DocumentationTargetSummary>,
}

/// A file attached to a documentation record. The UI does not always expose
/// a download URL or MIME type, so those fields intentionally remain optional.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DocumentationAttachment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// The expandable target summary rendered on a documentation detail page.
///
/// These are display references rather than backend records. IDs remain null
/// unless Fagbrev exposes them in the UI.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DocumentationTargetSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learning_plan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_text: Option<String>,
    #[serde(default)]
    pub competency_goals: Vec<CompetencyGoalReference>,
    #[serde(default)]
    pub delmal: Vec<Delmal>,
}

/// One page from the documentation table.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DocumentationPage {
    pub page: u32,
    pub page_size: u32,
    pub total: Option<u32>,
    pub items: Vec<DocumentationRecord>,
}

/// Documentation counts shown in the dashboard status summary.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DocumentationCounts {
    /// Items the dashboard describes as written or delivered.
    pub delivered: Option<u32>,
    pub in_review: Option<u32>,
    pub needing_correction: Option<u32>,
    pub approved: Option<u32>,
}

/// Focused status data derived from the broader dashboard overview.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DashboardStatus {
    pub progress_percent: Option<u8>,
    /// Kept separate from progress because the UI's meaning of “finished”
    /// still needs to be verified against the plan data.
    pub finished_percent: Option<u8>,
    pub documentation_counts: DocumentationCounts,
}

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

impl DashboardOverview {
    /// Convert the dashboard's current fields to the focused status contract.
    pub fn status(&self) -> DashboardStatus {
        DashboardStatus {
            progress_percent: self.progress_percent,
            finished_percent: None,
            documentation_counts: DocumentationCounts {
                delivered: self.documentations_written,
                in_review: self.documentations_to_review,
                needing_correction: self.documentations_needing_correction,
                approved: self.approved_documentations,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompetencyGoal {
    pub number: u8,
    pub title: String,
    pub status_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<DocumentationStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompetencyGoalDetails {
    pub number: u8,
    pub url: String,
    pub page_title: String,
    pub visible_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delmal: Vec<Delmal>,
}

/// A source item selected when searching for reusable learner-written content.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ReusableContentSource {
    pub documentation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Input contract for matching existing documentation to a new target.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReusableContentQuery {
    pub target: DocumentationTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
}

/// One explainable match returned by a reusable-content search.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReusableContentMatch {
    pub source: ReusableContentSource,
    pub target: DocumentationTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    #[serde(default)]
    pub reasons: Vec<String>,
}

/// Local draft output for a documentation entry. It is not a submission.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentationTemplate {
    pub target: DocumentationTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub content: String,
    #[serde(default)]
    pub source_documentation_ids: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_overview_fields_to_status_contract() {
        let overview = DashboardOverview {
            authenticated: true,
            page_title: "Fagbrev - Oversikt".to_string(),
            url: "https://fagbrev.io/l".to_string(),
            progress_percent: Some(38),
            documentations_written: Some(15),
            assessment_conversations: Some(4),
            documentations_needing_correction: Some(1),
            documentations_to_review: Some(0),
            approved_documentations: Some(14),
        };

        let status = overview.status();
        assert_eq!(status.progress_percent, Some(38));
        assert_eq!(status.finished_percent, None);
        assert_eq!(status.documentation_counts.delivered, Some(15));
        assert_eq!(status.documentation_counts.in_review, Some(0));
        assert_eq!(status.documentation_counts.needing_correction, Some(1));
        assert_eq!(status.documentation_counts.approved, Some(14));

        let encoded = serde_json::to_value(&status).expect("status should serialize");
        assert_eq!(encoded["finished_percent"], serde_json::Value::Null);
    }

    #[test]
    fn delmal_can_be_activity_text_without_an_independent_id() {
        let delmal = Delmal {
            goal_number: 1,
            number: Some(1),
            title: "Planlegge arbeidet".to_string(),
            id: None,
            status: None,
            status_count: None,
        };

        let encoded = serde_json::to_value(&delmal).expect("delmål should serialize");
        assert_eq!(encoded["goal_number"], 1);
        assert!(encoded.get("id").is_none());
    }
}
