//! Explicit, bounded context assembly for MCP callers.
//!
//! This module deliberately does not retain conversation memory. Each call
//! performs fresh reads and returns the limits, sources, and warnings that
//! explain exactly what was included.

use anyhow::{Result, bail};

use crate::{browser, data, drafts};

const DEFAULT_MAX_DOCUMENTS: u32 = 5;
const MAX_DOCUMENTS: u32 = 20;
const MAX_DOCUMENT_CONTENT_CHARS: usize = 12_000;
const TRUNCATION_SUFFIX: &str = "\n[… content truncated by context bundle …]";

#[derive(Debug, Clone)]
pub struct ContextRequest {
    pub goal_number: Option<u8>,
    pub document_ids: Option<Vec<String>>,
    pub max_documents: Option<u32>,
    pub include_document_content: bool,
}

pub async fn build(request: ContextRequest) -> Result<data::ContextBundle> {
    if let Some(goal_number) = request.goal_number
        && !(1..=21).contains(&goal_number)
    {
        bail!("goal_number must be between 1 and 21");
    }

    let requested_limit = request.max_documents.unwrap_or(DEFAULT_MAX_DOCUMENTS);
    let max_documents = requested_limit.clamp(1, MAX_DOCUMENTS);
    let mut warnings = Vec::new();
    if requested_limit > MAX_DOCUMENTS {
        warnings.push(format!(
            "max_documents was limited to {MAX_DOCUMENTS}; context bundles are bounded."
        ));
    }

    let dashboard = match browser::dashboard_overview().await {
        Ok(value) => Some(value),
        Err(error) => {
            warnings.push(format!("Could not read dashboard: {error:#}"));
            None
        }
    };
    let status = dashboard.as_ref().map(data::DashboardOverview::status);

    let mut source_urls = vec!["https://fagbrev.io/l".to_string()];
    let learning_plan = if let Some(goal_number) = request.goal_number {
        source_urls.push("https://fagbrev.io/l/laereplanmal".to_string());
        match browser::competency_goal(goal_number).await {
            Ok(goal) => Some(data::LearningPlanContext {
                goal_number,
                title: goal.title,
                status_count: goal.status_count,
                delmal: goal.delmal,
            }),
            Err(error) => {
                warnings.push(format!(
                    "Could not read competency goal {goal_number}: {error:#}"
                ));
                None
            }
        }
    } else {
        warnings.push(
            "No goal_number was selected; learning-plan details were omitted from this bundle."
                .to_string(),
        );
        None
    };

    let selected_ids = request.document_ids.is_some();
    let ids = request.document_ids.unwrap_or_default();
    for document_id in &ids {
        validate_documentation_id(document_id)?;
    }
    if ids.len() > max_documents as usize {
        warnings.push(format!(
            "Only the first {max_documents} selected documentation IDs were included."
        ));
    }

    let mut documentation = Vec::new();
    if selected_ids {
        for document_id in ids.into_iter().take(max_documents as usize) {
            match browser::get_documentation(&document_id).await {
                Ok(record) => {
                    if let Some(url) = record.url.clone() {
                        source_urls.push(url);
                    }
                    let (record, truncated) =
                        bound_document(record, request.include_document_content);
                    if truncated {
                        warnings.push(format!(
                            "Selected documentation {document_id} content was truncated to {MAX_DOCUMENT_CONTENT_CHARS} characters."
                        ));
                    }
                    documentation.push(record);
                }
                Err(error) => warnings.push(format!(
                    "Could not read selected documentation {document_id}: {error:#}"
                )),
            }
        }
    } else {
        let page_size = browser::documentation_page_size_for_limit(max_documents);
        match browser::list_documentation(1, page_size, None, None).await {
            Ok(page) => {
                if page.items.len() > max_documents as usize
                    || page.total.is_some_and(|total| total > max_documents)
                {
                    warnings.push(format!(
                        "Only the first {max_documents} documentation summaries were included; use document_ids for specific records."
                    ));
                }
                for record in page.items.into_iter().take(max_documents as usize) {
                    if let Some(url) = record.url.clone() {
                        source_urls.push(url);
                    }
                    let (record, _) = bound_document(record, false);
                    documentation.push(record);
                }
                if request.include_document_content {
                    warnings.push(
                        "include_document_content was ignored for the default archive slice; select document_ids to include content.".to_string(),
                    );
                }
            }
            Err(error) => {
                warnings.push(format!("Could not read documentation summaries: {error:#}"))
            }
        }
    }

    Ok(data::ContextBundle {
        retrieved_at: drafts::current_timestamp(),
        source_urls,
        limits: data::ContextBundleLimits {
            max_documents,
            include_document_content: request.include_document_content && selected_ids,
            selected_document_ids: selected_ids,
        },
        dashboard,
        status,
        learning_plan,
        documentation,
        warnings,
    })
}

fn bound_document(
    mut record: data::DocumentationRecord,
    include_document_content: bool,
) -> (data::DocumentationRecord, bool) {
    if !include_document_content {
        record.content = None;
        record.content_html = None;
        record.feedback.clear();
        record.attachments.clear();
        return (record, false);
    }
    let mut truncated = false;
    if let Some(content) = record.content.as_mut()
        && content.chars().count() > MAX_DOCUMENT_CONTENT_CHARS
    {
        *content = truncate(content, MAX_DOCUMENT_CONTENT_CHARS);
        truncated = true;
    }
    if let Some(content_html) = record.content_html.as_mut()
        && content_html.chars().count() > MAX_DOCUMENT_CONTENT_CHARS
    {
        *content_html = truncate(content_html, MAX_DOCUMENT_CONTENT_CHARS);
        truncated = true;
    }
    (record, truncated)
}

fn truncate(value: &str, max_chars: usize) -> String {
    let suffix_length = TRUNCATION_SUFFIX.chars().count();
    let content_length = max_chars.saturating_sub(suffix_length);
    let mut result = value.chars().take(content_length).collect::<String>();
    result.push_str(TRUNCATION_SUFFIX);
    result
}

fn validate_documentation_id(document_id: &str) -> Result<()> {
    if document_id.is_empty()
        || document_id.len() > 256
        || !document_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        bail!("document_ids must contain IDs from Fagbrev documentation links");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{bound_document, validate_documentation_id};
    use crate::data::{DocumentationRecord, DocumentationStatus};

    #[test]
    fn default_context_documents_are_summary_only() {
        let record = DocumentationRecord {
            id: Some("doc-1".to_string()),
            title: Some("Example".to_string()),
            content: Some("Private content".to_string()),
            content_html: Some("<p>Private content</p>".to_string()),
            status: DocumentationStatus::Draft,
            ..DocumentationRecord::default()
        };
        let (summary, truncated) = bound_document(record, false);
        assert!(summary.content.is_none());
        assert!(summary.content_html.is_none());
        assert!(!truncated);
    }

    #[test]
    fn context_document_ids_reject_path_like_values() {
        assert!(validate_documentation_id("doc-1").is_ok());
        assert!(validate_documentation_id("../secret").is_err());
    }

    #[test]
    fn selected_document_content_is_bounded() {
        let record = DocumentationRecord {
            content: Some("x".repeat(super::MAX_DOCUMENT_CONTENT_CHARS + 1)),
            ..DocumentationRecord::default()
        };
        let (bounded, truncated) = bound_document(record, true);
        assert!(truncated);
        assert_eq!(
            bounded
                .content
                .expect("content should remain")
                .chars()
                .count(),
            super::MAX_DOCUMENT_CONTENT_CHARS
        );
    }
}
