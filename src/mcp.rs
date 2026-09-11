use std::sync::Arc;

use anyhow::{Result, bail};
use rmcp::{
    ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::CallToolResult,
    schemars::JsonSchema,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::{Deserialize, Serialize};

use crate::browser;
use crate::data::{
    DocumentationStatus, DocumentationTarget, DocumentationWritePreview, RequestApprovalResult,
    SubmitDocumentationResult,
};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FagbrevServer {
    tool_router: ToolRouter<Self>,
    _marker: Arc<()>,
}

impl Default for FagbrevServer {
    fn default() -> Self {
        Self {
            tool_router: Self::tool_router(),
            _marker: Arc::new(()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoalRequest {
    /// Goal number, from 1 to 21.
    pub goal_number: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DelmalListRequest {
    /// Optional parent competency goal number. Omit to list delmål for all goals.
    #[serde(default)]
    pub goal_number: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DelmalRequest {
    /// Parent competency goal number, from 1 to 21.
    pub goal_number: u8,
    /// Ordinal within the parent goal. This is not a Firebase ID.
    pub delmal_number: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentationListRequest {
    /// 1-based table page. Defaults to 1.
    #[serde(default)]
    pub page: Option<u32>,
    /// Rows per page. Fagbrev currently exposes 6, 12, 18, 24, 100, and 200.
    #[serde(default)]
    pub page_size: Option<u32>,
    /// Optional text searched by the documentation table.
    #[serde(default)]
    pub query: Option<String>,
    /// Optional status filter exposed by the table.
    #[serde(default)]
    pub status: Option<DocumentationStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentationRequest {
    /// The real document ID copied from a /l/dokumentasjon/{id} link.
    pub document_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReusableContentRequest {
    /// New competency goal/delmål target to compare against.
    pub target: DocumentationTarget,
    /// Optional terms to require in source text in addition to target overlap.
    #[serde(default)]
    pub query: Option<String>,
    /// Optional allow-list of source documentation IDs.
    #[serde(default)]
    pub source_documentation_ids: Option<Vec<String>>,
    /// Optional source status filter.
    #[serde(default)]
    pub source_status: Option<DocumentationStatus>,
    /// Optional maximum number of returned suggestions.
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentationTemplateRequest {
    /// Target competency goal/delmål for the local draft.
    pub target: DocumentationTarget,
    /// Optional title for the local draft.
    #[serde(default)]
    pub title: Option<String>,
    /// Existing documentation IDs to include as clearly marked reference material.
    #[serde(default)]
    pub source_documentation_ids: Vec<String>,
    /// Optional instructions to place above the draft scaffold.
    #[serde(default)]
    pub extra_instructions: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubmitDocumentationRequest {
    /// The title shown for the new documentation entry.
    pub title: String,
    /// Plain text content. It is escaped before being placed in the rich-text editor.
    pub content: String,
    /// A competency goal, delmål, or matching pair of targets.
    pub target: DocumentationTarget,
    /// Must be true before the browser form is opened. Opening it may create a blank draft.
    #[serde(default)]
    pub confirm: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RequestApprovalRequest {
    /// The real document ID copied from a Fagbrev documentation link.
    pub document_id: String,
    /// Must be true before the visible approval action is clicked.
    #[serde(default)]
    pub confirm: bool,
}

const NEW_FORM_WARNING: &str = "Opening the new documentation form may create a blank draft. No form is opened and no browser data is changed until confirm=true.";

fn submit_preview(request: &SubmitDocumentationRequest) -> Result<DocumentationWritePreview> {
    browser::validate_documentation_target(&request.target)?;
    if request.title.trim().is_empty() {
        bail!("title cannot be empty");
    }
    if request.content.trim().is_empty() {
        bail!("content cannot be empty");
    }
    Ok(DocumentationWritePreview {
        operation: "submit_documentation".to_string(),
        confirmation_required: true,
        would_mutate: true,
        title: request.title.clone(),
        content: request.content.clone(),
        target: request.target.clone(),
        warning: NEW_FORM_WARNING.to_string(),
    })
}

fn approval_preview(document_id: String) -> RequestApprovalResult {
    RequestApprovalResult {
        outcome: "confirmation_required".to_string(),
        confirmation_required: true,
        would_mutate: true,
        mutated: false,
        document_id,
        url: None,
        status: None,
        message: "Preview only. If the record exposes the visible Send inn action, confirm=true will click it to request approval. No browser page was opened.".to_string(),
    }
}

#[tool_router]
impl FagbrevServer {
    /// Return the authenticated dashboard overview.
    #[tool(
        name = "get_dashboard_overview",
        description = "Read the current user's Fagbrev.io dashboard overview. Read-only."
    )]
    async fn get_dashboard_overview(&self) -> String {
        match browser::dashboard_overview().await {
            Ok(overview) => serde_json::to_string_pretty(&overview)
                .unwrap_or_else(|error| format!("Could not serialize dashboard: {error}")),
            Err(error) => format!("Could not read dashboard: {error:#}"),
        }
    }

    /// Return the focused status summary requested by the user.
    #[tool(
        name = "get_status",
        description = "Read the current user's Fagbrev.io progress and documentation status summary. Read-only; unavailable percentages remain null."
    )]
    async fn get_status(&self) -> String {
        match browser::dashboard_overview().await {
            Ok(overview) => serde_json::to_string_pretty(&overview.status())
                .unwrap_or_else(|error| format!("Could not serialize status: {error}")),
            Err(error) => format!("Could not read status: {error:#}"),
        }
    }

    /// List the 21 competency goals and their current documentation counts.
    #[tool(
        name = "list_competency_goals",
        description = "List the user's 21 Fagbrev.io competency goals with their visible status counts. Read-only."
    )]
    async fn list_competency_goals(&self) -> String {
        match browser::list_competency_goals().await {
            Ok(goals) => serde_json::to_string_pretty(&goals)
                .unwrap_or_else(|error| format!("Could not serialize competency goals: {error}")),
            Err(error) => format!("Could not read competency goals: {error:#}"),
        }
    }

    /// Return the selected competency goal and its visible sub-goals/work activities.
    #[tool(
        name = "get_competency_goal",
        description = "Read one competency goal and its sub-goals from the user's Fagbrev.io plan. Read-only."
    )]
    async fn get_competency_goal(
        &self,
        Parameters(request): Parameters<GoalRequest>,
    ) -> CallToolResult {
        if !(1..=21).contains(&request.goal_number) {
            return CallToolResult::error(vec![rmcp::model::ContentBlock::text(
                "goal_number must be between 1 and 21",
            )]);
        }

        match browser::competency_goal(request.goal_number).await {
            Ok(goal) => CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                serde_json::to_string_pretty(&goal).unwrap_or_else(|error| error.to_string()),
            )]),
            Err(error) => CallToolResult::error(vec![rmcp::model::ContentBlock::text(format!(
                "Could not read competency goal: {error:#}"
            ))]),
        }
    }

    /// List the selectable delmål/work activities exposed by the documentation target picker.
    #[tool(
        name = "list_delmal",
        description = "List selectable Fagbrev.io delmål, optionally filtered by parent competency goal. IDs and statuses remain null unless the UI exposes them. Read-only."
    )]
    async fn list_delmal(&self, Parameters(request): Parameters<DelmalListRequest>) -> String {
        match browser::list_delmal(request.goal_number).await {
            Ok(delmal) => serde_json::to_string_pretty(&delmal)
                .unwrap_or_else(|error| format!("Could not serialize delmål: {error}")),
            Err(error) => format!("Could not read delmål: {error:#}"),
        }
    }

    /// Return one selectable delmål by parent goal and UI ordinal.
    #[tool(
        name = "get_delmal",
        description = "Read one Fagbrev.io delmål by its parent competency goal and ordinal. The ordinal is UI-local and is not an invented backend ID. Read-only."
    )]
    async fn get_delmal(&self, Parameters(request): Parameters<DelmalRequest>) -> CallToolResult {
        match browser::get_delmal(request.goal_number, request.delmal_number).await {
            Ok(delmal) => CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                serde_json::to_string_pretty(&delmal).unwrap_or_else(|error| error.to_string()),
            )]),
            Err(error) => CallToolResult::error(vec![rmcp::model::ContentBlock::text(format!(
                "Could not read delmål: {error:#}"
            ))]),
        }
    }

    /// List documentation rows from the authenticated documentation table.
    #[tool(
        name = "list_documentation",
        description = "List the current user's Fagbrev.io documentation records with real IDs, dates, and UI-derived status. Read-only."
    )]
    async fn list_documentation(
        &self,
        Parameters(request): Parameters<DocumentationListRequest>,
    ) -> String {
        match browser::list_documentation(
            request.page.unwrap_or(1),
            request.page_size.unwrap_or(6),
            request.query,
            request.status,
        )
        .await
        {
            Ok(page) => serde_json::to_string_pretty(&page)
                .unwrap_or_else(|error| format!("Could not serialize documentation: {error}")),
            Err(error) => format!("Could not read documentation: {error:#}"),
        }
    }

    /// Read one documentation detail page without changing it.
    #[tool(
        name = "get_documentation",
        description = "Read one Fagbrev.io documentation record, including visible content, status, timestamps, attachments, and target summary. Read-only."
    )]
    async fn get_documentation(
        &self,
        Parameters(request): Parameters<DocumentationRequest>,
    ) -> CallToolResult {
        match browser::get_documentation(&request.document_id).await {
            Ok(documentation) => CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                serde_json::to_string_pretty(&documentation)
                    .unwrap_or_else(|error| error.to_string()),
            )]),
            Err(error) => CallToolResult::error(vec![rmcp::model::ContentBlock::text(format!(
                "Could not read documentation: {error:#}"
            ))]),
        }
    }

    /// Find existing learner documentation that may be useful reference material.
    #[tool(
        name = "find_reusable_content",
        description = "Find explainable local text-overlap suggestions from existing Fagbrev.io documentation for a target. Read-only; never copies or submits content."
    )]
    async fn find_reusable_content(
        &self,
        Parameters(request): Parameters<ReusableContentRequest>,
    ) -> String {
        match crate::reuse::find_reusable_content(
            request.target,
            request.query,
            request.source_documentation_ids,
            request.source_status,
            request.limit,
        )
        .await
        {
            Ok(result) => serde_json::to_string_pretty(&result).unwrap_or_else(|error| {
                format!("Could not serialize reusable-content matches: {error}")
            }),
            Err(error) => format!("Could not find reusable content: {error:#}"),
        }
    }

    /// Assemble a local draft from selected records without saving it.
    #[tool(
        name = "make_documentation_template",
        description = "Create a local reviewable documentation template from selected existing records. Includes marked source content and warnings; never saves, uploads, submits, or requests approval."
    )]
    async fn make_documentation_template(
        &self,
        Parameters(request): Parameters<DocumentationTemplateRequest>,
    ) -> String {
        match crate::reuse::make_documentation_template(
            request.target,
            request.title,
            request.source_documentation_ids,
            request.extra_instructions,
        )
        .await
        {
            Ok(template) => serde_json::to_string_pretty(&template).unwrap_or_else(|error| {
                format!("Could not serialize documentation template: {error}")
            }),
            Err(error) => format!("Could not make documentation template: {error:#}"),
        }
    }

    /// Save a new documentation entry only after explicit confirmation.
    #[tool(
        name = "submit_documentation",
        description = "Preview a new Fagbrev.io documentation entry by default. Only confirm=true may open the new-entry form and click Lagre; opening that form may itself create a blank draft. Uses visible UI controls only."
    )]
    async fn submit_documentation(
        &self,
        Parameters(request): Parameters<SubmitDocumentationRequest>,
    ) -> CallToolResult {
        let preview = match submit_preview(&request) {
            Ok(preview) => preview,
            Err(error) => {
                return CallToolResult::error(vec![rmcp::model::ContentBlock::text(
                    error.to_string(),
                )]);
            }
        };

        if !request.confirm {
            let response = SubmitDocumentationResult {
                outcome: "confirmation_required".to_string(),
                confirmation_required: true,
                mutated: false,
                preview,
                record: None,
                message: Some(NEW_FORM_WARNING.to_string()),
            };
            return CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                serde_json::to_string_pretty(&response).unwrap_or_else(|error| error.to_string()),
            )]);
        }

        match browser::submit_documentation(
            &request.title,
            &request.content,
            &request.target,
            request.confirm,
        )
        .await
        {
            Ok(record) => {
                let response = SubmitDocumentationResult {
                    outcome: "saved".to_string(),
                    confirmation_required: false,
                    mutated: true,
                    preview,
                    record: Some(record),
                    message: Some("The visible Lagre action was activated.".to_string()),
                };
                CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                    serde_json::to_string_pretty(&response)
                        .unwrap_or_else(|error| error.to_string()),
                )])
            }
            Err(error) => CallToolResult::error(vec![rmcp::model::ContentBlock::text(format!(
                "Could not save documentation through the visible UI: {error:#}"
            ))]),
        }
    }

    /// Request approval for an existing documentation record only after confirmation.
    #[tool(
        name = "request_approval",
        description = "Preview an approval request by default. Only confirm=true may click the visible Fagbrev.io Send inn action; if it is not exposed, returns unsupported without guessing an endpoint."
    )]
    async fn request_approval(
        &self,
        Parameters(request): Parameters<RequestApprovalRequest>,
    ) -> CallToolResult {
        if request.document_id.is_empty()
            || !request.document_id.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
        {
            return CallToolResult::error(vec![rmcp::model::ContentBlock::text(
                "document_id must be the ID from a Fagbrev documentation link",
            )]);
        }

        if !request.confirm {
            let response = approval_preview(request.document_id);
            return CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                serde_json::to_string_pretty(&response).unwrap_or_else(|error| error.to_string()),
            )]);
        }

        match browser::request_approval(&request.document_id, request.confirm).await {
            Ok(response) => CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                serde_json::to_string_pretty(&response).unwrap_or_else(|error| error.to_string()),
            )]),
            Err(error) => CallToolResult::error(vec![rmcp::model::ContentBlock::text(format!(
                "Could not request approval through the visible UI: {error:#}"
            ))]),
        }
    }
}

#[tool_handler]
impl rmcp::ServerHandler for FagbrevServer {}

pub async fn serve() -> Result<()> {
    let server = FagbrevServer::default();
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        RequestApprovalRequest, SubmitDocumentationRequest, approval_preview, submit_preview,
    };
    use crate::data::{CompetencyGoalReference, Delmal, DocumentationTarget};

    fn target() -> DocumentationTarget {
        DocumentationTarget {
            competency_goal: Some(CompetencyGoalReference {
                number: 15,
                title: Some("Feilsøke kode og rette feil".to_string()),
                id: None,
            }),
            delmal: Some(Delmal {
                goal_number: 15,
                number: Some(1),
                title: "Dokumentere virksomhetens rutiner for feilsøking".to_string(),
                id: None,
                status: None,
                status_count: None,
            }),
        }
    }

    #[test]
    fn submit_preview_is_confirmation_gated_and_serializable() {
        let request = SubmitDocumentationRequest {
            title: "Feilsøking".to_string(),
            content: "Jeg undersøkte og dokumenterte feilen.".to_string(),
            target: target(),
            confirm: false,
        };

        let preview = submit_preview(&request).expect("valid write request");
        assert!(preview.confirmation_required);
        assert!(preview.would_mutate);
        assert!(preview.warning.contains("blank draft"));
        let encoded = serde_json::to_value(&preview).expect("preview should serialize");
        assert_eq!(encoded["target"]["delmal"]["goal_number"], 15);
    }

    #[test]
    fn approval_preview_never_reports_mutation() {
        let response = approval_preview("document-123".to_string());
        assert!(response.confirmation_required);
        assert!(response.would_mutate);
        assert!(!response.mutated);
        assert_eq!(response.document_id, "document-123");
        let encoded = serde_json::to_value(response).expect("approval preview should serialize");
        assert_eq!(encoded["outcome"], "confirmation_required");
    }

    #[test]
    fn approval_request_defaults_to_preview() {
        let request: RequestApprovalRequest =
            serde_json::from_str(r#"{"document_id":"document-123"}"#)
                .expect("request should deserialize");
        assert!(!request.confirm);
    }
}
