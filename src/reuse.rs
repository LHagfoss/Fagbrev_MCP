//! Local composition over read-only documentation records.
//!
//! This module deliberately does not know how to save a draft. It loads
//! records through the existing browser adapter, then performs deterministic
//! text matching and assembles a reviewable local template.

use std::collections::BTreeSet;

use anyhow::{Result, bail};

use crate::{browser, data};

const MAX_EXCERPT_CHARS: usize = 240;
const PAGE_SIZE: u32 = 200;
const MAX_PAGES: u32 = 10;

/// Find documentation records whose text overlaps with the requested target
/// and optional query. Scores are proportions, not probabilities or semantic
/// similarity claims.
pub async fn find_reusable_content(
    target: data::DocumentationTarget,
    query: Option<String>,
    source_documentation_ids: Option<Vec<String>>,
    source_status: Option<data::DocumentationStatus>,
    limit: Option<u32>,
) -> Result<data::ReusableContentSearch> {
    validate_target(&target)?;
    let (records, mut warnings) = load_records(source_documentation_ids, source_status).await?;
    let matches = match_records(&target, &records, query.as_deref(), limit);

    warnings.push(
        "Matches use deterministic local text overlap and are suggestions only; review for actual relevance and rewrite in your own words.".to_string(),
    );

    Ok(data::ReusableContentSearch {
        target,
        matches,
        warnings,
    })
}

/// Retrieve selected records and assemble a local-only documentation draft.
/// No browser action beyond read-only detail pages is performed here.
pub async fn make_documentation_template(
    target: data::DocumentationTarget,
    title: Option<String>,
    source_documentation_ids: Vec<String>,
    extra_instructions: Option<String>,
) -> Result<data::DocumentationTemplate> {
    validate_target(&target)?;

    let mut records = Vec::new();
    let mut warnings = Vec::new();
    for source_id in &source_documentation_ids {
        match browser::get_documentation(source_id).await {
            Ok(record) => records.push(record),
            Err(error) => warnings.push(format!(
                "Could not read source documentation {source_id}: {error:#}"
            )),
        }
    }

    let mut template = compose_template(
        target,
        title,
        &source_documentation_ids,
        &records,
        extra_instructions.as_deref(),
    );
    template.warnings.splice(0..0, warnings);
    Ok(template)
}

fn validate_target(target: &data::DocumentationTarget) -> Result<()> {
    if target.competency_goal.is_none() && target.delmal.is_none() {
        bail!("target must include a competency_goal, a delmal, or both");
    }
    if target_terms(target).is_empty() {
        bail!("target must contain readable competency goal or delmål text");
    }
    Ok(())
}

async fn load_records(
    source_documentation_ids: Option<Vec<String>>,
    source_status: Option<data::DocumentationStatus>,
) -> Result<(Vec<data::DocumentationRecord>, Vec<String>)> {
    let requested_ids = source_documentation_ids.map(|ids| {
        ids.into_iter()
            .filter(|id| !id.trim().is_empty())
            .collect::<Vec<_>>()
    });
    let mut listed = Vec::new();
    let mut page_number = 1;
    let truncated;

    loop {
        let page = browser::list_documentation(page_number, PAGE_SIZE, None, source_status).await?;
        let page_len = page.items.len();
        listed.extend(page.items);

        let requested_found = requested_ids.as_ref().is_some_and(|requested| {
            requested.iter().all(|wanted| {
                listed
                    .iter()
                    .any(|record| record.id.as_deref() == Some(wanted.as_str()))
            })
        });
        let all_listed = page
            .total
            .is_none_or(|total| listed.len() >= total as usize);
        if page_len < PAGE_SIZE as usize
            || all_listed
            || requested_found
            || page_number >= MAX_PAGES
        {
            truncated = page_number >= MAX_PAGES && !all_listed && !requested_found;
            break;
        }
        page_number += 1;
    }

    let wanted: BTreeSet<&str> = requested_ids
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(String::as_str)
        .collect();
    let candidates = listed.into_iter().filter(|record| {
        wanted.is_empty() || record.id.as_deref().is_some_and(|id| wanted.contains(id))
    });

    let mut records = Vec::new();
    let mut warnings = Vec::new();
    if truncated {
        warnings.push(format!(
            "Stopped reading documentation after {MAX_PAGES} pages ({PAGE_SIZE} records per page)."
        ));
    }
    for summary in candidates {
        let Some(id) = summary.id.as_deref() else {
            warnings.push("Skipped a documentation row without a stable ID.".to_string());
            continue;
        };
        match browser::get_documentation(id).await {
            Ok(record) => records.push(record),
            Err(error) => warnings.push(format!(
                "Could not read source documentation {id}: {error:#}"
            )),
        }
    }

    if let Some(requested) = requested_ids {
        for id in requested {
            if !records
                .iter()
                .any(|record| record.id.as_deref() == Some(id.as_str()))
            {
                warnings.push(format!(
                    "Source documentation {id} was not found in the read-only documentation list."
                ));
            }
        }
    }

    Ok((records, warnings))
}

/// Normalize text into sorted, lower-case terms. Punctuation and common
/// connective words are ignored to keep matches explainable and stable.
pub(crate) fn normalize_tokens(text: &str) -> BTreeSet<String> {
    let mut terms = BTreeSet::new();
    let mut current = String::new();
    for character in text.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            current.push(character);
        } else if !current.is_empty() {
            add_term(&mut terms, &current);
            current.clear();
        }
    }
    if !current.is_empty() {
        add_term(&mut terms, &current);
    }
    terms
}

fn add_term(terms: &mut BTreeSet<String>, term: &str) {
    const STOP_WORDS: &[&str] = &[
        "a", "an", "and", "av", "den", "det", "en", "et", "for", "fra", "i", "in", "kan", "med",
        "of", "og", "on", "på", "som", "the", "til", "to", "å",
    ];
    if term.len() > 1 && !STOP_WORDS.contains(&term) {
        terms.insert(term.to_string());
    }
}

fn target_terms(target: &data::DocumentationTarget) -> BTreeSet<String> {
    let mut text = String::new();
    if let Some(goal) = &target.competency_goal {
        text.push_str(goal.title.as_deref().unwrap_or_default());
    }
    if let Some(delmal) = &target.delmal {
        text.push(' ');
        text.push_str(&delmal.title);
    }
    normalize_tokens(&text)
}

fn record_text(record: &data::DocumentationRecord) -> String {
    let mut text = String::new();
    if let Some(title) = &record.title {
        text.push_str(title);
        text.push(' ');
    }
    if let Some(content) = &record.content {
        text.push_str(content);
        text.push(' ');
    } else if let Some(content_html) = &record.content_html {
        text.push_str(content_html);
        text.push(' ');
    }
    if let Some(summary) = &record.target_summary {
        text.push_str(summary.summary_text.as_deref().unwrap_or_default());
        for goal in &summary.competency_goals {
            text.push(' ');
            text.push_str(goal.title.as_deref().unwrap_or_default());
        }
        for delmal in &summary.delmal {
            text.push(' ');
            text.push_str(&delmal.title);
        }
    }
    text
}

pub(crate) fn match_records(
    target: &data::DocumentationTarget,
    records: &[data::DocumentationRecord],
    query: Option<&str>,
    limit: Option<u32>,
) -> Vec<data::ReusableContentMatch> {
    let target_terms = target_terms(target);
    let query_terms = query.map(normalize_tokens).unwrap_or_default();
    let mut matches = records
        .iter()
        .filter_map(|record| {
            let source_terms = normalize_tokens(&record_text(record));
            let target_overlap: Vec<_> =
                target_terms.intersection(&source_terms).cloned().collect();
            let query_overlap: Vec<_> = query_terms.intersection(&source_terms).cloned().collect();
            let same_goal = target.competency_goal.as_ref().and_then(|goal| {
                record
                    .target_summary
                    .as_ref()?
                    .competency_goals
                    .iter()
                    .find(|source| source.number == goal.number)
            });
            if (target_overlap.is_empty() && same_goal.is_none())
                || (!query_terms.is_empty() && query_overlap.is_empty())
            {
                return None;
            }

            let target_score = if target_terms.is_empty() {
                0.0
            } else {
                target_overlap.len() as f32 / target_terms.len() as f32
            };
            let query_score = if query_terms.is_empty() {
                0.0
            } else {
                query_overlap.len() as f32 / query_terms.len() as f32
            };
            let mut score = if query_terms.is_empty() {
                target_score
            } else {
                target_score * 0.7 + query_score * 0.3
            };
            if same_goal.is_some() {
                score = score.max(0.25);
            }

            let mut reasons = Vec::new();
            if !target_overlap.is_empty() {
                reasons.push(format!(
                    "target text overlaps on: {}",
                    target_overlap.join(", ")
                ));
            }
            if !query_overlap.is_empty() {
                reasons.push(format!("query overlaps on: {}", query_overlap.join(", ")));
            }
            if let Some(goal) = same_goal {
                reasons.push(format!(
                    "source is attached to competency goal {}",
                    goal.number
                ));
            }

            let source_id = record.id.clone().unwrap_or_else(|| "unknown".to_string());
            let source_text = record_text(record);
            Some(data::ReusableContentMatch {
                source: data::ReusableContentSource {
                    documentation_id: source_id,
                    title: record.title.clone(),
                },
                target: record.target.clone(),
                score: Some((score * 1000.0).round() / 1000.0),
                excerpt: Some(short_excerpt(&source_text)),
                reasons,
            })
        })
        .collect::<Vec<_>>();

    matches.sort_by(|left, right| {
        right
            .score
            .unwrap_or_default()
            .total_cmp(&left.score.unwrap_or_default())
            .then_with(|| {
                left.source
                    .documentation_id
                    .cmp(&right.source.documentation_id)
            })
    });
    if let Some(limit) = limit {
        matches.truncate(limit as usize);
    }
    matches
}

fn short_excerpt(text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= MAX_EXCERPT_CHARS {
        return normalized;
    }
    let excerpt: String = normalized.chars().take(MAX_EXCERPT_CHARS).collect();
    format!("{excerpt}…")
}

pub(crate) fn compose_template(
    target: data::DocumentationTarget,
    title: Option<String>,
    selected_ids: &[String],
    records: &[data::DocumentationRecord],
    extra_instructions: Option<&str>,
) -> data::DocumentationTemplate {
    let mut warnings = vec![
        "Reference content is included for review only. Verify it, adapt it to this target, and write in your own words.".to_string(),
    ];
    if selected_ids.is_empty() {
        warnings.push(
            "No source documentation was selected; this is an empty local scaffold.".to_string(),
        );
    }

    let target_label = target_label(&target);
    let mut content = format!("# Documentation draft\n\nTarget: {target_label}\n\n");
    if let Some(instructions) = extra_instructions.filter(|text| !text.trim().is_empty()) {
        content.push_str("## Extra instructions\n\n");
        content.push_str(instructions.trim());
        content.push_str("\n\n");
    }
    content.push_str(
        "## Your draft\n\n[Describe what you did, how you did it, and the result or evidence.]\n\n",
    );
    content.push_str("## Reference material\n\n");

    for source_id in selected_ids {
        let Some(record) = records
            .iter()
            .find(|record| record.id.as_deref() == Some(source_id.as_str()))
        else {
            warnings.push(format!(
                "Source documentation {source_id} was not available."
            ));
            continue;
        };
        content.push_str(&format!("### SOURCE DOCUMENTATION {source_id}\n\n"));
        if let Some(title) = &record.title {
            content.push_str(&format!("Title: {title}\n\n"));
        }
        let source_content = record
            .content
            .as_deref()
            .or(record.content_html.as_deref())
            .unwrap_or("");
        if source_content.trim().is_empty() {
            warnings.push(format!(
                "Source documentation {source_id} has no readable text content."
            ));
        } else {
            content.push_str(source_content.trim());
            content.push_str("\n\n");
        }
        content.push_str(&format!("### END SOURCE DOCUMENTATION {source_id}\n\n"));
    }

    data::DocumentationTemplate {
        target,
        title,
        content,
        source_documentation_ids: selected_ids.to_vec(),
        warnings,
    }
}

fn target_label(target: &data::DocumentationTarget) -> String {
    let mut parts = Vec::new();
    if let Some(goal) = &target.competency_goal {
        let title = goal.title.as_deref().unwrap_or("(untitled)");
        parts.push(format!("competency goal {}: {title}", goal.number));
    }
    if let Some(delmal) = &target.delmal {
        let number = delmal
            .number
            .map(|number| format!("{}.", number))
            .unwrap_or_default();
        parts.push(format!("delmål {number} {}", delmal.title));
    }
    parts.join("; ")
}

#[cfg(test)]
mod tests {
    use super::{compose_template, match_records, normalize_tokens};
    use crate::data::{
        CompetencyGoalReference, Delmal, DocumentationRecord, DocumentationTarget,
        DocumentationTargetSummary,
    };

    fn target() -> DocumentationTarget {
        DocumentationTarget {
            competency_goal: Some(CompetencyGoalReference {
                number: 1,
                title: Some("Planlegge og utvikle løsninger".to_string()),
                id: None,
            }),
            delmal: Some(Delmal {
                goal_number: 1,
                number: Some(1),
                title: "Teste løsningen systematisk".to_string(),
                id: None,
                status: None,
                status_count: None,
            }),
        }
    }

    fn record(id: &str, title: &str, content: &str) -> DocumentationRecord {
        DocumentationRecord {
            id: Some(id.to_string()),
            title: Some(title.to_string()),
            content: Some(content.to_string()),
            target_summary: Some(DocumentationTargetSummary {
                competency_goals: vec![CompetencyGoalReference {
                    number: 1,
                    title: Some("Planlegge og utvikle løsninger".to_string()),
                    id: None,
                }],
                ..DocumentationTargetSummary::default()
            }),
            ..DocumentationRecord::default()
        }
    }

    #[test]
    fn normalizes_punctuation_case_and_stop_words() {
        let terms = normalize_tokens("Planlegge, OG teste løsningen!");
        assert!(terms.contains("planlegge"));
        assert!(terms.contains("teste"));
        assert!(terms.contains("løsningen"));
        assert!(!terms.contains("og"));
    }

    #[test]
    fn returns_explainable_deterministic_matches() {
        let matches = match_records(
            &target(),
            &[record(
                "doc-1",
                "Testing",
                "Jeg planlegge[d] and teste løsningen systematisk.",
            )],
            None,
            None,
        );
        assert_eq!(matches.len(), 1);
        assert!(matches[0].score.unwrap() > 0.0);
        assert!(
            matches[0]
                .reasons
                .iter()
                .any(|reason| reason.contains("overlaps"))
        );
        assert!(matches[0].excerpt.as_deref().unwrap().contains("løsningen"));
    }

    #[test]
    fn query_is_a_deterministic_source_text_filter() {
        let records = [
            record("doc-1", "Testing", "Jeg teste løsningen systematisk."),
            record("doc-2", "Planning", "Jeg planlegge løsningen."),
        ];
        let matches = match_records(&target(), &records, Some("systematisk"), None);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].source.documentation_id, "doc-1");
    }

    #[test]
    fn composes_selected_source_content_with_ids_and_extra_instructions() {
        let ids = vec!["doc-1".to_string()];
        let template = compose_template(
            target(),
            Some("Testing draft".to_string()),
            &ids,
            &[record("doc-1", "Testing", "Source evidence")],
            Some("Mention the test result."),
        );
        assert!(template.content.contains("SOURCE DOCUMENTATION doc-1"));
        assert!(template.content.contains("Source evidence"));
        assert!(template.content.contains("Mention the test result."));
        assert_eq!(template.source_documentation_ids, ids);
    }

    #[test]
    fn no_source_creates_scaffold_and_unknown_source_warns() {
        let empty = compose_template(target(), None, &[], &[], None);
        assert!(empty.content.contains("Your draft"));
        assert!(
            empty
                .warnings
                .iter()
                .any(|warning| warning.contains("No source"))
        );

        let unknown = compose_template(target(), None, &["missing".to_string()], &[], None);
        assert!(unknown.content.contains("Reference material"));
        assert!(
            unknown
                .warnings
                .iter()
                .any(|warning| warning.contains("missing"))
        );
    }
}
