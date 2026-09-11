use std::sync::Arc;

use anyhow::Result;
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
}

#[tool_handler]
impl rmcp::ServerHandler for FagbrevServer {}

pub async fn serve() -> Result<()> {
    let server = FagbrevServer::default();
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
