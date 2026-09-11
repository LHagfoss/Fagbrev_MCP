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
    DeleteDocumentationResult, DocumentationStatus, DocumentationTarget,
    DocumentationUpdatePreview, DocumentationWritePreview, RequestApprovalResult,
    SubmitDocumentationResult, UpdateDocumentationResult,
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
pub struct HalfYearTaskRequest {
    /// UI-local ordinal for one of the six half-year assignment areas.
    pub ordinal: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FeedbackListRequest {
    /// The real document ID copied from a Fagbrev documentation link.
    pub document_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FeedbackRequest {
    /// The real document ID copied from a Fagbrev documentation link.
    pub document_id: String,
    /// UI-local feedback ordinal, newest first; not a backend ID.
    pub feedback_ordinal: u32,
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
pub struct UpdateDocumentationRequest {
    /// The real document ID copied from a /l/dokumentasjon/{id} link.
    pub document_id: String,
    /// The replacement title shown by the documentation editor.
    pub title: String,
    /// Plain text replacement content for the rich-text editor.
    pub content: String,
    /// If provided, replace all visible competency-goal/delmål selections.
    /// Omit it to preserve the current target selection.
    #[serde(default)]
    pub replacement_target: Option<DocumentationTarget>,
    /// Exact `updated_at` value returned by `get_documentation`.
    pub expected_updated_at: String,
    /// Must be true before opening the edit form and clicking `Lagre`.
    #[serde(default)]
    pub confirm: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeleteDocumentationRequest {
    /// The real document ID copied from a /l/dokumentasjon/{id} link.
    pub document_id: String,
    /// Must explicitly be `draft`; approved and in-review records are refused.
    pub expected_status: DocumentationStatus,
    /// Exact `updated_at` value returned by `get_documentation`.
    pub expected_updated_at: String,
    /// Must be true before inspecting for and activating a visible delete control.
    #[serde(default)]
    pub confirm: bool,
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

const UPDATE_WARNING: &str = "No edit page is opened and no browser data changes until confirm=true. The expected_updated_at guard must still match the visible record before saving.";
const DELETE_WARNING: &str = "No documentation page is opened and nothing changes until confirm=true. Deletion is limited to drafts with an explicit status and updated_at guard; the current UI may report unsupported.";

fn valid_documentation_id(document_id: &str) -> Result<()> {
    if document_id.is_empty()
        || !document_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        bail!("document_id must be the ID from a Fagbrev documentation link");
    }
    Ok(())
}

fn update_preview(request: &UpdateDocumentationRequest) -> Result<DocumentationUpdatePreview> {
    valid_documentation_id(&request.document_id)?;
    if request.title.trim().is_empty() {
        bail!("title cannot be empty");
    }
    if request.content.trim().is_empty() {
        bail!("content cannot be empty");
    }
    if request.expected_updated_at.trim().is_empty() {
        bail!("expected_updated_at is required for safe documentation updates");
    }
    if let Some(target) = &request.replacement_target {
        browser::validate_documentation_target(target)?;
    }
    Ok(DocumentationUpdatePreview {
        operation: "update_documentation".to_string(),
        confirmation_required: true,
        would_mutate: true,
        document_id: request.document_id.clone(),
        title: request.title.clone(),
        content: request.content.clone(),
        replacement_target: request.replacement_target.clone(),
        expected_updated_at: request.expected_updated_at.clone(),
        warning: UPDATE_WARNING.to_string(),
    })
}

fn delete_preview(request: &DeleteDocumentationRequest) -> Result<DeleteDocumentationResult> {
    valid_documentation_id(&request.document_id)?;
    if request.expected_status != DocumentationStatus::Draft {
        bail!("delete_documentation only permits expected_status=draft");
    }
    if request.expected_updated_at.trim().is_empty() {
        bail!("expected_updated_at is required for safe documentation deletion");
    }
    Ok(DeleteDocumentationResult {
        outcome: "confirmation_required".to_string(),
        confirmation_required: true,
        would_mutate: true,
        mutated: false,
        document_id: request.document_id.clone(),
        expected_status: request.expected_status,
        expected_updated_at: request.expected_updated_at.clone(),
        status: None,
        message: DELETE_WARNING.to_string(),
    })
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

    /// List the six expandable half-year assignment areas.
    #[tool(
        name = "list_half_year_tasks",
        description = "List the six read-only Fagbrev.io half-year assignment and minifagprøve areas with their visible task text and documentation counts. Ordinals are UI-local."
    )]
    async fn list_half_year_tasks(&self) -> String {
        match browser::list_half_year_tasks().await {
            Ok(tasks) => serde_json::to_string_pretty(&tasks)
                .unwrap_or_else(|error| format!("Could not serialize half-year tasks: {error}")),
            Err(error) => format!("Could not read half-year tasks: {error:#}"),
        }
    }

    /// Read one half-year assignment area.
    #[tool(
        name = "get_half_year_task",
        description = "Read one of the six Fagbrev.io half-year assignment areas by UI-local ordinal. Read-only."
    )]
    async fn get_half_year_task(
        &self,
        Parameters(request): Parameters<HalfYearTaskRequest>,
    ) -> CallToolResult {
        match browser::get_half_year_task(request.ordinal).await {
            Ok(task) => CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                serde_json::to_string_pretty(&task).unwrap_or_else(|error| error.to_string()),
            )]),
            Err(error) => CallToolResult::error(vec![rmcp::model::ContentBlock::text(format!(
                "Could not read half-year task: {error:#}"
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

    /// List feedback entries visible on one documentation detail page.
    #[tool(
        name = "list_feedback",
        description = "List feedback entries visible on one authenticated Fagbrev.io documentation detail page. Feedback ordinals are UI-local and no private profile data is collected. Read-only."
    )]
    async fn list_feedback(&self, Parameters(request): Parameters<FeedbackListRequest>) -> String {
        match browser::list_feedback(&request.document_id).await {
            Ok(feedback) => serde_json::to_string_pretty(&feedback)
                .unwrap_or_else(|error| format!("Could not serialize feedback: {error}")),
            Err(error) => format!("Could not read feedback: {error:#}"),
        }
    }

    /// Read one feedback entry visible on a documentation detail page.
    #[tool(
        name = "get_feedback",
        description = "Read one visible feedback entry by documentation ID and UI-local newest-first ordinal. Read-only."
    )]
    async fn get_feedback(
        &self,
        Parameters(request): Parameters<FeedbackRequest>,
    ) -> CallToolResult {
        match browser::get_feedback(&request.document_id, request.feedback_ordinal).await {
            Ok(feedback) => CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                serde_json::to_string_pretty(&feedback).unwrap_or_else(|error| error.to_string()),
            )]),
            Err(error) => CallToolResult::error(vec![rmcp::model::ContentBlock::text(format!(
                "Could not read feedback: {error:#}"
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

    /// Edit an existing documentation entry only after explicit confirmation.
    #[tool(
        name = "update_documentation",
        description = "Preview an existing Fagbrev.io documentation update by default. Only confirm=true may open /l/dokumentasjon/{id}/rediger and click the exact visible Lagre control. Requires the exact updated_at value from get_documentation; an optional replacement_target replaces visible goal/delmål selections."
    )]
    async fn update_documentation(
        &self,
        Parameters(request): Parameters<UpdateDocumentationRequest>,
    ) -> CallToolResult {
        let preview = match update_preview(&request) {
            Ok(preview) => preview,
            Err(error) => {
                return CallToolResult::error(vec![rmcp::model::ContentBlock::text(
                    error.to_string(),
                )]);
            }
        };

        if !request.confirm {
            let response = UpdateDocumentationResult {
                outcome: "confirmation_required".to_string(),
                confirmation_required: true,
                mutated: false,
                preview,
                record: None,
                message: Some(UPDATE_WARNING.to_string()),
            };
            return CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                serde_json::to_string_pretty(&response).unwrap_or_else(|error| error.to_string()),
            )]);
        }

        match browser::update_documentation(
            &request.document_id,
            &request.title,
            &request.content,
            request.replacement_target.as_ref(),
            &request.expected_updated_at,
            request.confirm,
        )
        .await
        {
            Ok(record) => {
                let response = UpdateDocumentationResult {
                    outcome: "updated".to_string(),
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
                "Could not update documentation through the visible UI: {error:#}"
            ))]),
        }
    }

    /// Delete a draft only if the current UI exposes a clear delete control.
    #[tool(
        name = "delete_documentation",
        description = "Preview a draft deletion by default. Requires expected_status=draft and the exact updated_at value from get_documentation. A confirmed call inspects the visible detail page and returns unsupported unless it exposes an exact Slett control; it never guesses a backend delete endpoint."
    )]
    async fn delete_documentation(
        &self,
        Parameters(request): Parameters<DeleteDocumentationRequest>,
    ) -> CallToolResult {
        let preview = match delete_preview(&request) {
            Ok(preview) => preview,
            Err(error) => {
                return CallToolResult::error(vec![rmcp::model::ContentBlock::text(
                    error.to_string(),
                )]);
            }
        };

        if !request.confirm {
            return CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                serde_json::to_string_pretty(&preview).unwrap_or_else(|error| error.to_string()),
            )]);
        }

        match browser::delete_documentation(
            &request.document_id,
            request.expected_status,
            &request.expected_updated_at,
            request.confirm,
        )
        .await
        {
            Ok(response) => CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                serde_json::to_string_pretty(&response).unwrap_or_else(|error| error.to_string()),
            )]),
            Err(error) => CallToolResult::error(vec![rmcp::model::ContentBlock::text(format!(
                "Could not delete documentation through the visible UI: {error:#}"
            ))]),
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
        DeleteDocumentationRequest, RequestApprovalRequest, SubmitDocumentationRequest,
        UpdateDocumentationRequest, approval_preview, delete_preview, submit_preview,
        update_preview,
    };
    use crate::data::{CompetencyGoalReference, Delmal, DocumentationStatus, DocumentationTarget};

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

    #[test]
    fn update_preview_requires_concurrency_guard_and_preserves_target_omission() {
        let request = UpdateDocumentationRequest {
            document_id: "document-123".to_string(),
            title: "Updated title".to_string(),
            content: "Updated content".to_string(),
            replacement_target: None,
            expected_updated_at: "03.06.2026".to_string(),
            confirm: false,
        };

        let preview = update_preview(&request).expect("valid update request");
        assert!(preview.confirmation_required);
        assert!(preview.would_mutate);
        assert!(preview.replacement_target.is_none());
        assert!(preview.warning.contains("expected_updated_at"));

        let mut missing_guard = request;
        missing_guard.expected_updated_at.clear();
        let error = update_preview(&missing_guard).expect_err("guard is mandatory");
        assert!(error.to_string().contains("expected_updated_at"));
    }

    #[test]
    fn update_preview_validates_replacement_target_without_browser_access() {
        let request = UpdateDocumentationRequest {
            document_id: "document-123".to_string(),
            title: "Updated title".to_string(),
            content: "Updated content".to_string(),
            replacement_target: Some(target()),
            expected_updated_at: "03.06.2026".to_string(),
            confirm: false,
        };

        let preview = update_preview(&request).expect("target should validate");
        assert_eq!(preview.replacement_target, request.replacement_target);
    }

    #[test]
    fn delete_preview_is_draft_only_and_requires_explicit_version() {
        let request = DeleteDocumentationRequest {
            document_id: "document-123".to_string(),
            expected_status: DocumentationStatus::Draft,
            expected_updated_at: "11.09.2026".to_string(),
            confirm: false,
        };
        let preview = delete_preview(&request).expect("valid draft deletion request");
        assert_eq!(preview.outcome, "confirmation_required");
        assert!(preview.would_mutate);
        assert!(!preview.mutated);
        assert!(preview.message.contains("nothing changes"));

        let mut approved = request.clone();
        approved.expected_status = DocumentationStatus::Approved;
        let error = delete_preview(&approved).expect_err("approved records are protected");
        assert!(error.to_string().contains("expected_status=draft"));

        let mut missing_version = request;
        missing_version.expected_updated_at.clear();
        let error = delete_preview(&missing_version).expect_err("version is mandatory");
        assert!(error.to_string().contains("expected_updated_at"));
    }
}
