use std::{
    env,
    io::{self, Write},
    path::PathBuf,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use chromiumoxide::{Browser, browser::BrowserConfig};
use directories::ProjectDirs;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::time::{sleep, timeout};

use crate::data::{
    CompetencyGoal, CompetencyGoalDetails, CompetencyGoalReference, DashboardOverview, Delmal,
    DocumentationAttachment, DocumentationPage, DocumentationRecord, DocumentationStatus,
    DocumentationTarget, DocumentationTargetSummary, FeedbackRecord, HalfYearTask,
    LinkedDocumentationSummary, RequestApprovalResult,
};

const FAGBREV_URL: &str = "https://fagbrev.io/l";
const PLAN_URL: &str = "https://fagbrev.io/l/laereplanmal";
const DOCUMENTATION_URL: &str = "https://fagbrev.io/l/dokumentasjon";
const LOGIN_WAIT: Duration = Duration::from_secs(10 * 60);
const HALF_YEAR_TASK_TITLES: [&str; 6] = [
    "Etikk, lovverk og yrkesutøvelse",
    "Kodeferdigheter og metode",
    "Sikkerhet og personvern",
    "Infrastruktur og arkitektur",
    "Design, interaksjon og brukerdialog",
    "Minifagprøve",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PageSnapshot {
    title: String,
    url: String,
    text: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DocumentationRowSnapshot {
    id: Option<String>,
    url: Option<String>,
    title: Option<String>,
    updated_at: Option<String>,
    status_icon: Option<String>,
    status_color: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct DocumentationListSnapshot {
    page_size: Option<u32>,
    total: Option<u32>,
    rows: Vec<DocumentationRowSnapshot>,
}

#[derive(Debug, Clone, Deserialize)]
struct DocumentationTableReadiness {
    has_table: bool,
    linked_rows: u32,
    body_text: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DocumentationDetailSnapshot {
    title: Option<String>,
    status_label: Option<String>,
    status_icon: Option<String>,
    status_color: Option<String>,
    updated_at: Option<String>,
    content_text: Option<String>,
    content_html: Option<String>,
    attachments: Vec<DocumentationAttachment>,
    target_text: Option<String>,
    feedback: Vec<FeedbackSnapshot>,
}

#[derive(Debug, Clone, Deserialize)]
struct FeedbackSnapshot {
    status_label: Option<String>,
    status_icon: Option<String>,
    status_color: Option<String>,
    author: Option<String>,
    timestamp: Option<String>,
    text: String,
}

#[derive(Debug, Clone, Deserialize)]
struct HalfYearTaskSnapshot {
    ordinal: u8,
    title: String,
    task_text: Option<String>,
    status_label: Option<String>,
    status_icon: Option<String>,
    status_color: Option<String>,
    documentation_count: Option<u32>,
    linked_documentation: Vec<LinkedDocumentationSummary>,
}

#[derive(Debug, Clone, Deserialize)]
struct UiMutationResult {
    ok: bool,
    #[serde(default)]
    missing: Vec<String>,
    #[serde(default)]
    message: Option<String>,
}

fn app_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("io", "fagbrev", "fagbrev-mcp")
        .context("could not determine an OS application-data directory")
}

fn profile_dir() -> Result<PathBuf> {
    Ok(app_dirs()?.data_dir().join("browser-profile"))
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn browser_config(headful: bool) -> Result<BrowserConfig> {
    let profile = profile_dir()?;
    std::fs::create_dir_all(&profile)
        .with_context(|| format!("could not create browser profile at {}", profile.display()))?;

    let mut builder = BrowserConfig::builder()
        .user_data_dir(profile)
        .port(0)
        .window_size(1280, 900);

    if let Ok(executable) = env::var("FAGBREV_BROWSER_EXECUTABLE") {
        builder = builder.chrome_executable(executable);
    }

    if headful {
        builder = builder.with_head();
    }

    builder
        .build()
        .map_err(|error| anyhow::anyhow!("could not build Chromium configuration: {error}"))
}

async fn launch(headful: bool) -> Result<(Browser, chromiumoxide::Handler)> {
    if let Ok(cdp_url) = env::var("FAGBREV_CDP_URL") {
        if cdp_url.trim().is_empty() {
            bail!("FAGBREV_CDP_URL is set but empty");
        }
        tracing::info!(%cdp_url, "attaching to an existing browser over CDP");
        return Browser::connect(cdp_url).await.map_err(Into::into);
    }

    Browser::launch(browser_config(headful)?)
        .await
        .map_err(|error| {
            anyhow::anyhow!(
                "could not launch Chromium: {error}. Install Chrome/Chromium or set FAGBREV_CDP_URL to an existing browser's WebSocket URL"
            )
        })
}

async fn page_snapshot(page: &chromiumoxide::Page) -> Result<PageSnapshot> {
    let title = page.get_title().await?.unwrap_or_default();
    let url = page.url().await?.unwrap_or_default();
    let text: String = page
        .evaluate("() => document.body ? document.body.innerText : ''")
        .await?
        .into_value()?;

    Ok(PageSnapshot { title, url, text })
}

async fn wait_for_text(page: &chromiumoxide::Page, needle: &str) -> Result<PageSnapshot> {
    let mut snapshot = page_snapshot(page).await?;
    for _ in 0..20 {
        if snapshot.text.contains(needle) {
            return Ok(snapshot);
        }
        sleep(Duration::from_millis(500)).await;
        snapshot = page_snapshot(page).await?;
    }
    Ok(snapshot)
}

fn is_authenticated(snapshot: &PageSnapshot) -> bool {
    let text = snapshot.text.to_lowercase();
    let url = snapshot.url.to_lowercase();
    (url.contains("fagbrev.io/l") || url.contains("fagbrev.io/"))
        && (text.contains("velkommen") || text.contains("opplæringsplan"))
        && !text.contains("telefon eller e-post")
}

fn number_after(text: &str, marker: &str) -> Option<u32> {
    let words = text.split_whitespace().collect::<Vec<_>>();
    let index = words
        .iter()
        .position(|word| word.trim_matches(|c: char| !c.is_alphabetic()) == marker)?;
    words
        .get(index + 1)?
        .trim_matches(|c: char| !c.is_ascii_digit())
        .parse()
        .ok()
}

fn number_before_phrase(text: &str, phrase: &str) -> Option<u32> {
    let words = text.split_whitespace().collect::<Vec<_>>();
    let phrase_words = phrase.split_whitespace().collect::<Vec<_>>();
    let start = words
        .windows(phrase_words.len())
        .position(|window| window == phrase_words)?;
    words
        .get(start.checked_sub(1)?)?
        .trim_matches(|c: char| !c.is_ascii_digit())
        .parse()
        .ok()
}

fn dashboard_overview_from_snapshot(snapshot: PageSnapshot) -> DashboardOverview {
    DashboardOverview {
        authenticated: is_authenticated(&snapshot),
        page_title: snapshot.title,
        url: snapshot.url,
        progress_percent: number_after(&snapshot.text, "fullført")
            .and_then(|value| u8::try_from(value).ok()),
        documentations_written: number_after(&snapshot.text, "skrevet"),
        assessment_conversations: number_after(&snapshot.text, "hatt"),
        documentations_needing_correction: number_before_phrase(
            &snapshot.text,
            "Dokumentasjoner som må rettes",
        ),
        documentations_to_review: number_before_phrase(
            &snapshot.text,
            "Dokumentasjoner til vurdering",
        ),
        approved_documentations: number_before_phrase(&snapshot.text, "Godkjente dokumentasjoner"),
    }
}

fn documentation_status_from_ui(
    label: Option<&str>,
    icon: Option<&str>,
    color: Option<&str>,
) -> DocumentationStatus {
    let label = label.unwrap_or_default().trim().to_lowercase();
    let icon = icon.unwrap_or_default().trim().to_lowercase();
    let color = color.unwrap_or_default().trim().to_lowercase();

    if label == "godkjent" || icon == "check-circle" || color == "bg-positive" {
        return DocumentationStatus::Approved;
    }
    if label == "må rettes"
        || label == "trenger endring"
        || (icon == "edit-2" && color == "bg-negative")
    {
        return DocumentationStatus::NeedsCorrection;
    }
    if label == "til vurdering" || icon == "clock" {
        return DocumentationStatus::InReview;
    }
    if label == "kladd" || (icon == "edit" && color == "bg-dark") {
        return DocumentationStatus::Draft;
    }
    if label == "avvist" {
        return DocumentationStatus::Rejected;
    }

    DocumentationStatus::Unknown
}

fn parse_feedback(document_id: &str, entries: Vec<FeedbackSnapshot>) -> Vec<FeedbackRecord> {
    entries
        .into_iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let text = entry.text.trim().to_string();
            (!text.is_empty()).then_some(FeedbackRecord {
                ordinal: index as u32 + 1,
                document_id: document_id.to_string(),
                status: Some(documentation_status_from_ui(
                    entry.status_label.as_deref(),
                    entry.status_icon.as_deref(),
                    entry.status_color.as_deref(),
                ))
                .filter(|status| *status != DocumentationStatus::Unknown),
                author: entry
                    .author
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
                timestamp: entry
                    .timestamp
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
                text,
            })
        })
        .collect()
}

fn parse_half_year_tasks(entries: Vec<HalfYearTaskSnapshot>) -> Vec<HalfYearTask> {
    entries
        .into_iter()
        .map(|entry| {
            let status = documentation_status_from_ui(
                entry.status_label.as_deref(),
                entry.status_icon.as_deref(),
                entry.status_color.as_deref(),
            );
            HalfYearTask {
                ordinal: entry.ordinal,
                title: entry.title,
                task_text: entry
                    .task_text
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
                status: (status != DocumentationStatus::Unknown).then_some(status),
                documentation_count: entry.documentation_count,
                linked_documentation: entry.linked_documentation,
            }
        })
        .collect()
}

fn parse_documentation_rows(rows: Vec<DocumentationRowSnapshot>) -> Vec<DocumentationRecord> {
    rows.into_iter()
        .map(|row| DocumentationRecord {
            id: row.id,
            url: row.url,
            title: row.title,
            updated_at: row.updated_at,
            status: documentation_status_from_ui(
                None,
                row.status_icon.as_deref(),
                row.status_color.as_deref(),
            ),
            ..DocumentationRecord::default()
        })
        .collect()
}

fn parse_target_summary(text: Option<&str>) -> Option<DocumentationTargetSummary> {
    let text = text?.trim();
    if text.is_empty() {
        return None;
    }

    let mut summary = DocumentationTargetSummary {
        summary_text: Some(text.to_string()),
        ..DocumentationTargetSummary::default()
    };
    let mut current_goal = None;
    let mut next_delmal = 0u16;
    let mut saw_target_data = false;

    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line.eq_ignore_ascii_case("LÆREPLAN") {
            continue;
        }
        if summary.learning_plan.is_none() && line.starts_with("Læreplan") {
            summary.learning_plan = Some(line.to_string());
            continue;
        }
        if line.contains("Kompetansemål") {
            summary.category = Some(line.to_string());
            continue;
        }
        if line.contains("mål og") && line.contains("delmål") {
            continue;
        }
        if line.eq_ignore_ascii_case("Ingen kompetansemål tilknyttet") {
            continue;
        }
        if let Some((number, title)) = numbered_heading(line)
            && (1..=21).contains(&number)
        {
            summary.competency_goals.push(CompetencyGoalReference {
                number,
                title: Some(title.to_string()),
                id: None,
            });
            current_goal = Some(number);
            next_delmal = 0;
            saw_target_data = true;
            continue;
        }
        if line.ends_with("dok.") || line.starts_with("Halvårsoppgaver") {
            continue;
        }
        if let Some(goal_number) = current_goal {
            next_delmal += 1;
            summary.delmal.push(Delmal {
                goal_number,
                number: Some(next_delmal),
                title: line.to_string(),
                id: None,
                status: None,
                status_count: None,
            });
            saw_target_data = true;
        }
    }

    (saw_target_data || summary.learning_plan.is_some() || summary.category.is_some())
        .then_some(summary)
}

fn documentation_record_from_detail(
    id: String,
    url: String,
    snapshot: DocumentationDetailSnapshot,
) -> DocumentationRecord {
    DocumentationRecord {
        id: Some(id.clone()),
        url: Some(url),
        title: snapshot.title,
        content: snapshot
            .content_text
            .filter(|value| !value.trim().is_empty()),
        content_html: snapshot
            .content_html
            .filter(|value| !value.trim().is_empty()),
        status: documentation_status_from_ui(
            snapshot.status_label.as_deref(),
            snapshot.status_icon.as_deref(),
            snapshot.status_color.as_deref(),
        ),
        updated_at: snapshot.updated_at,
        attachments: snapshot.attachments,
        target_summary: parse_target_summary(snapshot.target_text.as_deref()),
        feedback: parse_feedback(&id, snapshot.feedback),
        ..DocumentationRecord::default()
    }
}

async fn open_dashboard(
    headful: bool,
) -> Result<(
    Browser,
    tokio::task::JoinHandle<()>,
    chromiumoxide::Page,
    bool,
)> {
    let owns_browser = env::var_os("FAGBREV_CDP_URL").is_none();
    let (mut browser, mut handler) = launch(headful).await?;
    let handler_task = tokio::spawn(async move {
        while let Some(event) = handler.next().await {
            if let Err(error) = event {
                tracing::debug!(%error, "browser handler stopped");
                break;
            }
        }
    });
    let page = if owns_browser {
        browser.new_page(FAGBREV_URL).await?
    } else {
        browser.fetch_targets().await?;
        sleep(Duration::from_millis(250)).await;
        let mut pages = browser.pages().await?;
        if let Some(page) = pages.pop() {
            page
        } else {
            bail!("connected browser has no open pages")
        }
    };
    Ok((browser, handler_task, page, owns_browser))
}

async fn plan_page(
    headful: bool,
) -> Result<(
    Browser,
    tokio::task::JoinHandle<()>,
    chromiumoxide::Page,
    bool,
)> {
    let session = open_dashboard(headful).await?;
    if env::var_os("FAGBREV_CDP_URL").is_none() {
        wait_for_text(&session.2, "Opplæringsplan").await?;
        session.2.goto(PLAN_URL).await?;
        let snapshot = wait_for_text(&session.2, "KOMPETANSEMÅL").await?;
        if !snapshot.text.contains("KOMPETANSEMÅL") {
            bail!("the competency-goal tab did not finish loading");
        }
    } else if !session
        .2
        .url()
        .await?
        .unwrap_or_default()
        .contains("/l/laereplanmal")
    {
        bail!("attached browser is not on the læreplan page; navigate there first");
    }
    Ok(session)
}

async fn half_year_page(
    headful: bool,
) -> Result<(
    Browser,
    tokio::task::JoinHandle<()>,
    chromiumoxide::Page,
    bool,
)> {
    let session = plan_page(headful).await?;
    wait_for_text(&session.2, "HALVÅRSOPPGAVER").await?;
    let selected = session
        .2
        .evaluate(
            "() => { const tab = Array.from(document.querySelectorAll('[role=\"tab\"], button')).find(element => (element.textContent || '').replace(/\\s+/g, ' ').toLowerCase().includes('halvårsoppgaver')); if (!tab) return false; if (tab.getAttribute('aria-selected') !== 'true') tab.click(); return true; }",
        )
        .await?
        .into_value::<bool>()?;
    if !selected {
        let current = page_snapshot(&session.2).await?;
        finish(session.0, session.1, session.3).await?;
        bail!(
            "could not find the half-year assignments tab on {} ({})",
            current.url,
            current.title
        );
    }
    let snapshot = wait_for_text(&session.2, "Minifagprøve").await?;
    if !snapshot.text.contains("Minifagprøve") {
        finish(session.0, session.1, session.3).await?;
        bail!("the half-year assignments tab did not finish loading");
    }
    Ok(session)
}

async fn half_year_tasks_snapshot(page: &chromiumoxide::Page) -> Result<Vec<HalfYearTaskSnapshot>> {
    let titles = serde_json::to_string(&HALF_YEAR_TASK_TITLES)?;
    page.evaluate(format!(
        r#"() => {{
            const titles = {titles};
            const normalize = value => (value || '').replace(/\s+/g, ' ').trim();
            const buttons = Array.from(document.querySelectorAll('main button.FExpantion-trigger'));
            titles.forEach(expected => {{
                const button = buttons.find(candidate => {{
                    const heading = candidate.querySelector('.FExpantion-header');
                    const title = normalize((heading?.innerText || candidate.innerText).replace(/^\s*\d+\s*/, ''));
                    return title === expected;
                }});
                if (button && !button.parentElement?.querySelector('.FExpantion-arrow-open')) button.click();
            }});
            return true;
        }}"#
    ))
    .await?
    .into_value::<bool>()?;
    sleep(Duration::from_millis(400)).await;
    page.evaluate(format!(
        r#"() => {{
            const titles = {titles};
            const normalize = value => (value || '').replace(/\s+/g, ' ').trim();
            const buttons = Array.from(document.querySelectorAll('main button.FExpantion-trigger'));
            return titles.map((expected, index) => {{
                const button = buttons.find(candidate => {{
                    const heading = candidate.querySelector('.FExpantion-header');
                    const title = normalize((heading?.innerText || candidate.innerText).replace(/^\s*\d+\s*/, ''));
                    return title === expected;
                }});
                if (!button) return null;
                const root = button.parentElement;
                const heading = button.querySelector('.FExpantion-header');
                const title = normalize((heading?.innerText || button.innerText).replace(/^\s*\d+\s*/, ''));
                const statusIcon = root?.querySelector('.FExpantion-header i[data-name], .FExpantion-header i[data-type]');
                const documentationButton = Array.from(root?.querySelectorAll('button') || [])
                    .find(candidate => normalize(candidate.innerText).startsWith('Se tilknyttede dokumentasjoner'));
                const documentationMatch = documentationButton?.innerText.match(/\((\d+)\)/);
                const editor = root?.querySelector('.FExpantion-content .ql-editor');
                const linked = Array.from(root?.querySelectorAll('a[href^="/l/dokumentasjon/"]') || [])
                    .map(anchor => {{
                        const href = anchor.getAttribute('href');
                        const match = href?.match(/^\/l\/dokumentasjon\/([^/?#]+)$/);
                        return {{ id: match ? match[1] : null, title: normalize(anchor.innerText) || null, status: null }};
                    }});
                return {{
                    ordinal: index + 1,
                    title,
                    task_text: editor?.innerText?.trim() || null,
                    status_label: null,
                    status_icon: statusIcon?.getAttribute('data-name') || statusIcon?.getAttribute('data-type') || null,
                    status_color: null,
                    documentation_count: documentationMatch ? Number(documentationMatch[1]) : null,
                    linked_documentation: linked
                }};
            }}).filter(Boolean);
        }}"#
    ))
    .await?
    .into_value()
    .map_err(Into::into)
}

fn parse_goals_from_page_text(text: &str) -> Vec<CompetencyGoal> {
    let mut goals = Vec::new();
    let mut pending_status = None;

    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Ok(status_count) = line.parse::<u32>() {
            pending_status = Some(status_count);
            continue;
        }

        let Some((number, title)) = line.split_once(". ") else {
            continue;
        };
        let Ok(number) = number.parse::<u8>() else {
            continue;
        };
        let Some(status_count) = pending_status.take() else {
            continue;
        };
        if (1..=21).contains(&number) {
            goals.push(CompetencyGoal {
                number,
                title: title.to_string(),
                status_count,
                id: None,
                status: None,
            });
        }
    }

    goals
}

fn numbered_heading(line: &str) -> Option<(u8, &str)> {
    let (number, title) = line.split_once(". ")?;
    let number = number.parse::<u8>().ok()?;
    let title = title.trim();
    (!title.is_empty()).then_some((number, title))
}

fn push_expanded_delmal(goal_number: u8, delmal: &mut Vec<Delmal>, current: &mut Option<String>) {
    if let Some(title) = current.take()
        && !title.ends_with(':')
    {
        delmal.push(Delmal {
            goal_number,
            number: Some(delmal.len() as u16 + 1),
            title,
            id: None,
            status: None,
            status_count: None,
        });
    }
}

fn parse_expanded_goal_from_page_text(
    text: &str,
    goal_number: u8,
) -> (Option<String>, Option<u32>, Vec<Delmal>) {
    let mut title = None;
    let mut status_count = None;
    let mut in_goal = false;
    let mut delmal = Vec::new();
    let mut current_title: Option<String> = None;
    let mut pending_status_count = None;

    for raw_line in text.lines().filter(|line| !line.trim().is_empty()) {
        let line = raw_line.trim();
        if let Some((number, heading)) = numbered_heading(line) {
            if number == goal_number {
                in_goal = true;
                title = Some(heading.to_string());
                status_count = pending_status_count.take();
                current_title = None;
                continue;
            }
            if in_goal && (1..=21).contains(&number) {
                break;
            }
        }

        if !in_goal {
            if let Ok(value) = line.parse::<u32>() {
                pending_status_count = Some(value);
            }
            continue;
        }

        if line.starts_with("Se tilknyttede dokumentasjoner")
            || line == "Ny dokumentasjon"
            || line.starts_with("Godkjent ")
            || line.starts_with("Til vurdering")
            || line.starts_with("Trenger endring")
            || line.starts_with("Last ned ")
        {
            break;
        }

        if current_title.is_none() && line.parse::<u32>().is_ok() {
            status_count = line.parse::<u32>().ok();
            continue;
        }

        if let Some(activity) = line.strip_prefix("* ") {
            push_expanded_delmal(goal_number, &mut delmal, &mut current_title);
            let activity = activity.trim();
            if !activity.ends_with(':') {
                current_title = Some(activity.to_string());
            }
            continue;
        }

        if raw_line.chars().next().is_some_and(char::is_whitespace) {
            if let Some(current) = current_title.as_mut() {
                current.push(' ');
                current.push_str(line);
            }
        } else {
            // Unbulleted, non-indented text is a section heading or prose,
            // not another part of the preceding work activity.
            push_expanded_delmal(goal_number, &mut delmal, &mut current_title);
        }
    }

    push_expanded_delmal(goal_number, &mut delmal, &mut current_title);

    (title, status_count, delmal)
}

async fn documentation_picker_page(
    headful: bool,
) -> Result<(
    Browser,
    tokio::task::JoinHandle<()>,
    chromiumoxide::Page,
    bool,
)> {
    let session = open_dashboard(headful).await?;
    let page = &session.2;
    page.goto(DOCUMENTATION_URL).await?;
    wait_for_text(page, "Skriv ny dokumentasjon").await?;

    let opened: bool = page
        .evaluate(
            "() => { const button = Array.from(document.querySelectorAll('button')).find(button => (button.textContent || '').includes('Skriv ny dokumentasjon')); if (!button) return false; button.click(); return true; }",
        )
        .await?
        .into_value()?;
    if !opened {
        finish(session.0, session.1, session.3).await?;
        bail!("could not open the new documentation form");
    }

    wait_for_text(page, "Kompetansemål og vurdering").await?;
    let opened_competency_section: bool = page
        .evaluate(
            "() => { const button = Array.from(document.querySelectorAll('button')).find(button => (button.textContent || '').includes('Kompetansemål og vurdering')); if (!button) return false; button.click(); return true; }",
        )
        .await?
        .into_value()?;
    if !opened_competency_section {
        finish(session.0, session.1, session.3).await?;
        bail!("could not open the competency-goal target picker");
    }

    wait_for_text(page, "valgbare").await?;
    for _ in 0..8 {
        let expanded: bool = page
            .evaluate(
                "() => { const buttons = Array.from(document.querySelectorAll('button')).filter(button => /^Vis \\d+ til$/.test((button.textContent || '').trim())); buttons.forEach(button => button.click()); return buttons.length > 0; }",
            )
            .await?
            .into_value()?;
        if !expanded {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }

    Ok(session)
}

async fn click_documentation_expansion(page: &chromiumoxide::Page, prefix: &str) -> Result<bool> {
    let prefix = serde_json::to_string(prefix)?;
    page.evaluate(format!(
        "() => {{ const button = Array.from(document.querySelectorAll('button')).find(button => (button.textContent || '').trim().startsWith({prefix})); if (!button) return false; button.click(); return true; }}"
    ))
    .await?
    .into_value()
    .map_err(Into::into)
}

async fn documentation_list_snapshot(
    page: &chromiumoxide::Page,
) -> Result<DocumentationListSnapshot> {
    page.evaluate(
        r#"() => {
            const rows = Array.from(document.querySelectorAll('table tr'))
                .filter(row => row.querySelectorAll('td').length >= 4)
                .map(row => {
                    const cells = row.querySelectorAll('td');
                    const href = row.querySelector('a[href^="/l/dokumentasjon/"]')?.getAttribute('href') || null;
                    const match = href && href.match(/^\/l\/dokumentasjon\/([^/?#]+)$/);
                    const icon = cells[0]?.querySelector('i[data-name], i[data-type]');
                    const statusContainer = cells[0]?.querySelector('[class*="bg-"]');
                    return {
                        id: match ? match[1] : null,
                        url: href,
                        title: cells[1]?.innerText.trim() || null,
                        updated_at: cells[2]?.innerText.trim() || null,
                        status_icon: icon?.getAttribute('data-name') || icon?.getAttribute('data-type') || null,
                status_color: statusContainer?.className.match(/(?:^|\s)(bg-[^\s]+)/)?.[1] || null
                    };
                });
            const total_match = document.body.innerText.match(/\bav\s+(\d+)\b/i);
            const page_size = document.querySelector('input[role="combobox"]')?.value;
            return {
                page_size: page_size ? Number(page_size) : null,
                total: total_match ? Number(total_match[1]) : null,
                rows
            };
        }"#,
    )
    .await?
    .into_value()
    .map_err(Into::into)
}

fn documentation_table_is_rendered(readiness: &DocumentationTableReadiness) -> bool {
    if !readiness.has_table || readiness.linked_rows > 0 {
        return readiness.has_table;
    }

    let lower = readiness.body_text.to_lowercase();
    let has_total = lower
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|pair| pair[0] == "av" && pair[1].parse::<u32>().is_ok());
    let has_empty_state = ["ingen data", "ingen dokumentasjoner", "ingen treff"]
        .iter()
        .any(|marker| lower.contains(marker));

    has_total || has_empty_state
}

async fn wait_for_documentation_rows(page: &chromiumoxide::Page) -> Result<()> {
    for _ in 0..20 {
        let readiness: DocumentationTableReadiness = page
            .evaluate(
                "() => ({ has_table: !!document.querySelector('table'), linked_rows: document.querySelectorAll('table a[href^=\"/l/dokumentasjon/\"]').length, body_text: document.body?.innerText || '' })",
            )
            .await?
            .into_value()?;
        if documentation_table_is_rendered(&readiness) {
            return Ok(());
        }
        sleep(Duration::from_millis(250)).await;
    }
    bail!("documentation table did not finish loading")
}

async fn documentation_detail_snapshot(
    page: &chromiumoxide::Page,
) -> Result<DocumentationDetailSnapshot> {
    page.evaluate(
        r#"() => {
            const main = document.querySelector('main');
            const knownStatuses = ['Kladd', 'Godkjent', 'Må rettes', 'Trenger endring', 'Til vurdering', 'Avvist'];
            const statusParagraph = Array.from(main?.querySelectorAll('p') || [])
                .find(element => knownStatuses.includes(element.innerText.trim()));
            const statusIcon = Array.from(main?.querySelectorAll('i[data-name], i[data-type]') || [])
                .find(element => ['check-circle', 'edit', 'edit-2', 'clock'].includes(element.getAttribute('data-name') || element.getAttribute('data-type')));
            const colorContainer = statusIcon?.closest('[class*="bg-"]');
            const updated = Array.from(main?.querySelectorAll('small') || [])
                .find(element => element.innerText.trim().startsWith('Sist endret:'));
            const editors = Array.from(main?.querySelectorAll('.ql-editor') || []);
            const content = editors.find(element => !element.classList.contains('fb-editor')) || editors[0];
            const attachmentButton = Array.from(main?.querySelectorAll('button') || [])
                .find(button => /^Vedlegg \(\d+\)$/.test(button.innerText.trim()));
            const attachmentPanel = attachmentButton?.closest('.FExpantion');
            const attachments = Array.from(attachmentPanel?.querySelectorAll('.FFileViewerBtn-content span') || [])
                .map(element => element.innerText.trim())
                .filter(Boolean)
                .map(name => ({name, url: null, mime_type: null}));
            const targetButton = Array.from(main?.querySelectorAll('button') || [])
                .find(button => button.innerText.trim().startsWith('Se kompetansemål'));
            const targetPanel = targetButton?.closest('.FExpantion');
            const feedbackCard = Array.from(main?.querySelectorAll('.vcard') || [])
                .find(card => Array.from(card.querySelectorAll('h3')).some(heading => heading.innerText.trim() === 'Tilbakemeldinger'));
            const feedback = Array.from(feedbackCard?.querySelectorAll('.divide-y > div') || [])
                .map(item => {
                    const badge = item.querySelector('.FBadge');
                    const icon = badge?.querySelector('i[data-name], i[data-type]');
                    return {
                        status_label: badge?.innerText.trim() || null,
                        status_icon: icon?.getAttribute('data-name') || icon?.getAttribute('data-type') || null,
                        status_color: null,
                        author: item.querySelector('span')?.innerText.trim() || null,
                        timestamp: item.querySelector('small')?.innerText.trim() || null,
                        text: item.querySelector('p')?.innerText.trim() || ''
                    };
                })
            return {
                title: main?.querySelector('h3')?.innerText.trim() || null,
                status_label: statusParagraph?.innerText.trim() || null,
                status_icon: statusIcon?.getAttribute('data-name') || statusIcon?.getAttribute('data-type') || null,
                status_color: colorContainer?.className.match(/(?:^|\s)(bg-[^\s]+)/)?.[1] || null,
                updated_at: updated?.innerText.replace(/^Sist endret:\s*/, '').trim() || null,
                content_text: content?.innerText.trim() || null,
                content_html: content?.innerHTML || null,
                attachments,
                target_text: targetPanel?.innerText.trim() || null,
                feedback
            };
        }"#,
    )
    .await?
    .into_value()
    .map_err(Into::into)
}

async fn wait_for_feedback_section(page: &chromiumoxide::Page) -> Result<()> {
    for _ in 0..20 {
        let rendered: bool = page
            .evaluate(
                "() => { const card = Array.from(document.querySelectorAll('main .vcard')).find(element => Array.from(element.querySelectorAll('h3')).some(heading => heading.innerText.trim() === 'Tilbakemeldinger')); if (!card) return false; return !!card.querySelector('.divide-y > div') || (card.innerText || '').includes('Ingen tilbakemeldinger'); }",
            )
            .await?
            .into_value()?;
        if rendered {
            return Ok(());
        }
        sleep(Duration::from_millis(250)).await;
    }
    Ok(())
}

pub async fn list_documentation(
    page_number: u32,
    page_size: u32,
    query: Option<String>,
    status: Option<DocumentationStatus>,
) -> Result<DocumentationPage> {
    if page_number == 0 {
        bail!("page must be at least 1");
    }
    if !matches!(page_size, 6 | 12 | 18 | 24 | 100 | 200) {
        bail!("page_size must be one of 6, 12, 18, 24, 100, or 200");
    }

    let (browser, handler_task, page, owns_browser) = open_dashboard(false).await?;
    page.goto(DOCUMENTATION_URL).await?;
    wait_for_text(&page, "En oversikt over alle dine dokumentasjoner").await?;
    wait_for_documentation_rows(&page).await?;

    if let Some(query) = query.filter(|value| !value.trim().is_empty()) {
        let value = serde_json::to_string(&query)?;
        page.evaluate(format!(
            "() => {{ const input = document.querySelector('input[aria-label=\\\"Søk\\\"]'); if (!input) return false; const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set; setter.call(input, {value}); input.dispatchEvent(new Event('input', {{bubbles: true}})); input.dispatchEvent(new Event('change', {{bubbles: true}})); return true; }}"
        ))
        .await?
        .into_value::<bool>()?;
        sleep(Duration::from_millis(500)).await;
        wait_for_documentation_rows(&page).await?;
    }

    if let Some(status) = status {
        let label = match status {
            DocumentationStatus::Draft => "Kladd",
            DocumentationStatus::NeedsCorrection => "Må rettes",
            DocumentationStatus::Approved => "Godkjent",
            DocumentationStatus::Submitted | DocumentationStatus::InReview => "Til vurdering",
            DocumentationStatus::Rejected => "Avvist",
            DocumentationStatus::Unknown => "Alle",
        };
        let opened = page
            .evaluate(
                "() => { const button = Array.from(document.querySelectorAll('button')).find(button => (button.textContent || '').includes('Vis') && (button.textContent || '').includes('Alle')); if (!button) return false; button.click(); return true; }",
            )
            .await?
            .into_value::<bool>()?;
        if !opened {
            finish(browser, handler_task, owns_browser).await?;
            bail!("could not open the documentation status filter");
        }
        let label = serde_json::to_string(label)?;
        let selected = page
            .evaluate(format!(
                "() => {{ const item = Array.from(document.querySelectorAll('.q-menu *')).find(element => (element.textContent || '').trim().startsWith({label})); if (!item) return false; item.click(); return true; }}"
            ))
            .await?
            .into_value::<bool>()?;
        if !selected {
            finish(browser, handler_task, owns_browser).await?;
            bail!("documentation status filter is not exposed by the current UI");
        }
        sleep(Duration::from_millis(500)).await;
        wait_for_documentation_rows(&page).await?;
    }

    if page_size != 6 {
        page.evaluate(
            "() => { const input = document.querySelector('input[role=\"combobox\"]'); if (!input) return false; input.click(); return true; }",
        )
        .await?
        .into_value::<bool>()?;
        sleep(Duration::from_millis(100)).await;
        let page_size_label = serde_json::to_string(&page_size.to_string())?;
        let selected = page
            .evaluate(format!(
                "() => {{ const item = Array.from(document.querySelectorAll('[role=\"option\"]')).find(element => (element.textContent || '').trim() === {page_size_label}); if (!item) return false; item.click(); return true; }}"
            ))
            .await?
            .into_value::<bool>()?;
        if !selected {
            finish(browser, handler_task, owns_browser).await?;
            bail!("page size option is not exposed by the current UI");
        }
        sleep(Duration::from_millis(500)).await;
        wait_for_documentation_rows(&page).await?;
    }

    if page_number > 1 {
        let page_label = serde_json::to_string(&page_number.to_string())?;
        let selected = page
            .evaluate(format!(
                "() => {{ const item = Array.from(document.querySelectorAll('nav button')).find(element => (element.textContent || '').trim() === {page_label}); if (!item) return false; item.click(); return true; }}"
            ))
            .await?
            .into_value::<bool>()?;
        if !selected {
            finish(browser, handler_task, owns_browser).await?;
            bail!("documentation page {page_number} is not available");
        }
        sleep(Duration::from_millis(500)).await;
        wait_for_documentation_rows(&page).await?;
    }

    let snapshot = documentation_list_snapshot(&page).await?;
    let actual_page_size = snapshot.page_size.unwrap_or(page_size);
    let items = parse_documentation_rows(snapshot.rows);
    let result = DocumentationPage {
        page: page_number,
        page_size: actual_page_size,
        total: snapshot.total,
        items,
    };
    finish(browser, handler_task, owns_browser).await?;
    Ok(result)
}

pub async fn get_documentation(document_id: &str) -> Result<DocumentationRecord> {
    if document_id.is_empty()
        || !document_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        bail!("document_id must be the ID from a Fagbrev documentation link");
    }

    let (browser, handler_task, page, owns_browser) = open_dashboard(false).await?;
    let url = format!("{DOCUMENTATION_URL}/{document_id}");
    page.goto(&url).await?;
    wait_for_text(&page, "Sist endret:").await?;
    wait_for_feedback_section(&page).await?;
    let _ = click_documentation_expansion(&page, "Vedlegg (").await?;
    let _ = click_documentation_expansion(&page, "Se kompetansemål").await?;
    sleep(Duration::from_millis(250)).await;
    let snapshot = documentation_detail_snapshot(&page).await?;
    let result = documentation_record_from_detail(document_id.to_string(), url, snapshot);
    finish(browser, handler_task, owns_browser).await?;
    Ok(result)
}

/// Validate a target before a write is allowed to reach the browser UI.
///
/// Delmål are UI-local references in the current site, so a write requires
/// both the parent goal number and the visible ordinal/title. No backend ID is
/// inferred here.
pub fn validate_documentation_target(target: &DocumentationTarget) -> Result<()> {
    if target.competency_goal.is_none() && target.delmal.is_none() {
        bail!("target must include a competency_goal, a delmal, or both");
    }

    if let Some(goal) = &target.competency_goal {
        if !(1..=21).contains(&goal.number) {
            bail!("competency_goal.number must be between 1 and 21");
        }
        if goal
            .title
            .as_deref()
            .is_some_and(|title| title.trim().is_empty())
        {
            bail!("competency_goal.title cannot be empty when provided");
        }
    }

    if let Some(delmal) = &target.delmal {
        if !(1..=21).contains(&delmal.goal_number) {
            bail!("delmal.goal_number must be between 1 and 21");
        }
        if delmal.number.is_none_or(|number| number == 0) {
            bail!("delmal.number is required and must be greater than zero");
        }
        if delmal.title.trim().is_empty() {
            bail!("delmal.title cannot be empty");
        }
        if target
            .competency_goal
            .as_ref()
            .is_some_and(|goal| goal.number != delmal.goal_number)
        {
            bail!("competency_goal.number must match delmal.goal_number");
        }
    }

    Ok(())
}

fn plain_text_as_safe_html(content: &str) -> String {
    content
        .split("\n\n")
        .map(|paragraph| {
            let escaped = paragraph
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
                .replace('\'', "&#39;")
                .replace('\n', "<br>");
            format!("<p>{escaped}</p>")
        })
        .collect::<Vec<_>>()
        .join("")
}

fn documentation_id_from_url(url: &str) -> Option<String> {
    let id = url.split("/l/dokumentasjon/").nth(1)?.split('/').next()?;
    (!id.is_empty()
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_')))
    .then(|| id.to_string())
}

async fn fill_new_documentation_form(
    page: &chromiumoxide::Page,
    title: &str,
    content: &str,
    target: &DocumentationTarget,
) -> Result<()> {
    let title = serde_json::to_string(title)?;
    let content_html = serde_json::to_string(&plain_text_as_safe_html(content))?;
    let mut targets = Vec::new();
    if let Some(goal) = &target.competency_goal {
        targets.push(serde_json::json!({
            "kind": "competency_goal",
            "number": goal.number,
            "title": goal.title,
        }));
    }
    if let Some(delmal) = &target.delmal {
        targets.push(serde_json::json!({
            "kind": "delmal",
            "goal_number": delmal.goal_number,
            "number": delmal.number,
            "title": delmal.title,
        }));
    }
    let targets = serde_json::to_string(&targets)?;

    let result: UiMutationResult = page
        .evaluate(format!(
            r#"() => {{
                const targets = {targets};
                const normalize = value => (value || '').replace(/\\s+/g, ' ').trim();
                const labels = Array.from(document.querySelectorAll('main label'));
                const found = targets.map(target => ({{ target, label: null }}));
                let currentGoal = null;
                let delmalOrdinal = 0;

                for (const label of labels) {{
                    const text = normalize(label.innerText);
                    const checkbox = label.querySelector('.FCheckbox');
                    const goalMatch = text.match(/^(\\d+)\\.\\s+/);
                    if (checkbox && goalMatch) {{
                        currentGoal = Number(goalMatch[1]);
                        delmalOrdinal = 0;
                        for (const item of found.filter(item => item.target.kind === 'competency_goal')) {{
                            const titleMatches = !item.target.title || text.startsWith(`${{item.target.number}}. ${{item.target.title}}`);
                            if (currentGoal === item.target.number && titleMatches) item.label = label;
                        }}
                        continue;
                    }}
                    if (!checkbox || currentGoal === null) continue;
                    delmalOrdinal += 1;
                    for (const item of found.filter(item => item.target.kind === 'delmal')) {{
                        const titleMatches = !item.target.title || text === normalize(item.target.title);
                        if (currentGoal === item.target.goal_number && delmalOrdinal === item.target.number && titleMatches) item.label = label;
                    }}
                }}

                const missing = found.filter(item => !item.label).map(item => item.target.kind);
                if (missing.length) return {{ok: false, missing, message: 'one or more target references were not found in the visible picker'}};

                const input = document.querySelector('input[placeholder="Dokumentasjonavn"]');
                const editor = document.querySelector('main .ql-editor');
                if (!input || !editor) return {{ok: false, missing: [], message: 'the documentation title or editor field was not exposed by the UI'}};
                const inputSetter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
                inputSetter.call(input, {title});
                input.dispatchEvent(new Event('input', {{bubbles: true}}));
                input.dispatchEvent(new Event('change', {{bubbles: true}}));
                editor.innerHTML = {content_html};
                editor.dispatchEvent(new InputEvent('input', {{bubbles: true, inputType: 'insertText'}}));
                editor.dispatchEvent(new Event('change', {{bubbles: true}}));
                found.forEach(item => item.label.click());
                return {{ok: true, missing: [], message: null}};
            }}"#
        ))
        .await?
        .into_value()?;

    if !result.ok {
        let mut detail = result
            .message
            .unwrap_or_else(|| "the documentation form could not be prepared".to_string());
        if !result.missing.is_empty() {
            detail.push_str(&format!(" (missing: {})", result.missing.join(", ")));
        }
        bail!(detail);
    }
    Ok(())
}

/// Create/save a documentation entry through the visible Fagbrev form.
///
/// This function is intentionally only called by the MCP layer after
/// `confirm: true`. Opening this form itself may create a blank draft.
pub async fn submit_documentation(
    title: &str,
    content: &str,
    target: &DocumentationTarget,
    confirm: bool,
) -> Result<DocumentationRecord> {
    if !confirm {
        bail!("submit_documentation requires confirm=true before opening the form");
    }
    validate_documentation_target(target)?;
    if title.trim().is_empty() {
        bail!("title cannot be empty");
    }
    if content.trim().is_empty() {
        bail!("content cannot be empty");
    }

    let (browser, handler_task, page, owns_browser) = documentation_picker_page(false).await?;
    let result = async {
        fill_new_documentation_form(&page, title, content, target).await?;
        let clicked: bool = page
            .evaluate(
                "() => { const button = Array.from(document.querySelectorAll('main button')).find(button => button.type === 'submit' && (button.textContent || '').trim() === 'Lagre'); if (!button) return false; button.click(); return true; }",
            )
            .await?
            .into_value()?;
        if !clicked {
            bail!("the documentation save control was not exposed by the UI");
        }

        sleep(Duration::from_millis(750)).await;
        let url = page.url().await?.unwrap_or_default();
        let id = documentation_id_from_url(&url);
        Ok(DocumentationRecord {
            id,
            url: (!url.is_empty()).then_some(url),
            title: Some(title.to_string()),
            content: Some(content.to_string()),
            content_html: Some(plain_text_as_safe_html(content)),
            status: DocumentationStatus::Draft,
            target: target.clone(),
            ..DocumentationRecord::default()
        })
    }
    .await;
    finish(browser, handler_task, owns_browser).await?;
    result
}

/// Request approval using the visible `Send inn` action on a documentation
/// detail page. If that action is absent, no click is attempted.
pub async fn request_approval(document_id: &str, confirm: bool) -> Result<RequestApprovalResult> {
    if !confirm {
        bail!("request_approval requires confirm=true before opening a documentation page");
    }
    if document_id.is_empty()
        || !document_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        bail!("document_id must be the ID from a Fagbrev documentation link");
    }

    let (browser, handler_task, page, owns_browser) = open_dashboard(false).await?;
    let url = format!("{DOCUMENTATION_URL}/{document_id}");
    page.goto(&url).await?;
    wait_for_text(&page, "Sist endret:").await?;
    let result = async {
        let clicked: bool = page
            .evaluate(
                "() => { const button = Array.from(document.querySelectorAll('main button')).find(button => (button.textContent || '').trim() === 'Send inn'); if (!button) return false; button.click(); return true; }",
            )
            .await?
            .into_value()?;
        if !clicked {
            return Ok(RequestApprovalResult {
                outcome: "unsupported".to_string(),
                confirmation_required: false,
                would_mutate: false,
                mutated: false,
                document_id: document_id.to_string(),
                url: Some(url.clone()),
                status: None,
                message: "The current documentation page does not expose a safe 'Send inn' approval action; nothing was changed.".to_string(),
            });
        }

        sleep(Duration::from_millis(750)).await;
        let snapshot = page_snapshot(&page).await?;
        let status = if snapshot.text.contains("Til vurdering") {
            Some(DocumentationStatus::InReview)
        } else {
            None
        };
        Ok(RequestApprovalResult {
            outcome: "requested".to_string(),
            confirmation_required: false,
            would_mutate: true,
            mutated: true,
            document_id: document_id.to_string(),
            url: Some(snapshot.url),
            status,
            message: "The visible 'Send inn' action was activated.".to_string(),
        })
    }
    .await;
    finish(browser, handler_task, owns_browser).await?;
    result
}

pub async fn dashboard_overview() -> Result<DashboardOverview> {
    let (browser, handler_task, page, owns_browser) = open_dashboard(false).await?;
    if !page
        .url()
        .await?
        .unwrap_or_default()
        .trim_end_matches('/')
        .ends_with("fagbrev.io/l")
    {
        page.goto(FAGBREV_URL).await?;
        sleep(Duration::from_millis(750)).await;
    }
    let mut snapshot = page_snapshot(&page).await?;
    for _ in 0..15 {
        if is_authenticated(&snapshot) {
            break;
        }
        sleep(Duration::from_millis(500)).await;
        snapshot = page_snapshot(&page).await?;
    }
    let overview = dashboard_overview_from_snapshot(snapshot);
    finish(browser, handler_task, owns_browser).await?;
    Ok(overview)
}

pub async fn list_competency_goals() -> Result<Vec<CompetencyGoal>> {
    let (browser, handler_task, page, owns_browser) = plan_page(false).await?;
    let goals = parse_goals_from_page_text(&page_snapshot(&page).await?.text);
    finish(browser, handler_task, owns_browser).await?;
    Ok(goals)
}

pub async fn competency_goal(number: u8) -> Result<CompetencyGoalDetails> {
    if !(1..=21).contains(&number) {
        bail!("goal number must be between 1 and 21");
    }

    let (browser, handler_task, page, owns_browser) = plan_page(false).await?;
    let clicked: bool = page
        .evaluate(format!(
            "() => {{ const button = Array.from(document.querySelectorAll('button')).find(button => new RegExp('(?:^|\\\\s){}\\\\.\\\\s').test((button.textContent || '').replace(/\\\\s+/g, ' ').trim())); if (!button) return false; button.click(); return true; }}",
            number
        ))
        .await?
        .into_value()?;

    if !clicked {
        finish(browser, handler_task, owns_browser).await?;
        bail!("could not find competency goal {number} on the plan page");
    }

    sleep(Duration::from_millis(500)).await;
    let snapshot = page_snapshot(&page).await?;
    let (title, status_count, delmal) = parse_expanded_goal_from_page_text(&snapshot.text, number);
    let result = CompetencyGoalDetails {
        number,
        url: snapshot.url,
        page_title: snapshot.title,
        visible_text: snapshot.text,
        title,
        status_count,
        delmal,
    };
    finish(browser, handler_task, owns_browser).await?;
    Ok(result)
}

pub async fn list_delmal(goal_number: Option<u8>) -> Result<Vec<Delmal>> {
    if let Some(number) = goal_number
        && !(1..=21).contains(&number)
    {
        bail!("goal number must be between 1 and 21");
    }

    // The documentation target picker is not read-only: opening a new-entry
    // form has been observed to create a blank draft. Read delmål from the
    // existing training-plan UI instead.
    let goal_numbers = goal_number.map_or_else(|| (1..=21).collect(), |number| vec![number]);
    let mut result = Vec::new();
    for number in goal_numbers {
        result.extend(competency_goal(number).await?.delmal);
    }
    Ok(result)
}

pub async fn get_delmal(goal_number: u8, delmal_number: u16) -> Result<Delmal> {
    if !(1..=21).contains(&goal_number) {
        bail!("goal number must be between 1 and 21");
    }
    let delmal = list_delmal(Some(goal_number))
        .await?
        .into_iter()
        .find(|item| item.number == Some(delmal_number));
    delmal.ok_or_else(|| {
        anyhow::anyhow!("could not find delmål {delmal_number} for goal {goal_number}")
    })
}

pub async fn list_half_year_tasks() -> Result<Vec<HalfYearTask>> {
    let (browser, handler_task, page, owns_browser) = half_year_page(false).await?;
    let result = async {
        let snapshot = half_year_tasks_snapshot(&page).await?;
        Ok::<Vec<HalfYearTask>, anyhow::Error>(parse_half_year_tasks(snapshot))
    }
    .await;
    finish(browser, handler_task, owns_browser).await?;
    result
}

pub async fn get_half_year_task(ordinal: u8) -> Result<HalfYearTask> {
    if !(1..=6).contains(&ordinal) {
        bail!("half-year task ordinal must be between 1 and 6");
    }
    list_half_year_tasks()
        .await?
        .into_iter()
        .find(|task| task.ordinal == ordinal)
        .ok_or_else(|| anyhow::anyhow!("could not find half-year task {ordinal}"))
}

pub async fn list_feedback(document_id: &str) -> Result<Vec<FeedbackRecord>> {
    Ok(get_documentation(document_id).await?.feedback)
}

pub async fn get_feedback(document_id: &str, ordinal: u32) -> Result<FeedbackRecord> {
    if ordinal == 0 {
        bail!("feedback ordinal must be greater than zero");
    }
    list_feedback(document_id)
        .await?
        .into_iter()
        .find(|feedback| feedback.ordinal == ordinal)
        .ok_or_else(|| {
            anyhow::anyhow!("could not find feedback {ordinal} for document {document_id}")
        })
}

async fn finish(
    mut browser: Browser,
    handler_task: tokio::task::JoinHandle<()>,
    close_browser: bool,
) -> Result<()> {
    if close_browser {
        browser.close().await?;
        browser.wait().await?;
    }
    handler_task.abort();
    Ok(())
}

pub async fn login() -> Result<()> {
    let profile = profile_dir()?;
    println!("Opening Fagbrev.io in a dedicated browser profile.");
    println!("Profile: {}", profile.display());
    println!(
        "Complete the normal login in the browser window; no credentials are read by this CLI."
    );

    let (browser, handler_task, page, owns_browser) = open_dashboard(true).await?;
    let result = timeout(LOGIN_WAIT, async {
        loop {
            let snapshot = page_snapshot(&page).await?;
            if is_authenticated(&snapshot) {
                return Ok::<PageSnapshot, anyhow::Error>(snapshot);
            }
            sleep(Duration::from_secs(2)).await;
        }
    })
    .await;

    let output = match result {
        Ok(Ok(snapshot)) => {
            println!("Login detected on the authenticated dashboard.");
            print_json(&dashboard_overview_from_snapshot(snapshot))
        }
        Ok(Err(error)) => Err(error),
        Err(_) => bail!("timed out waiting for login; the browser profile was preserved"),
    };

    finish(browser, handler_task, owns_browser).await?;
    output
}

pub async fn status() -> Result<()> {
    print_json(&dashboard_overview().await?)
}

pub async fn inspect() -> Result<()> {
    let (browser, handler_task, page, owns_browser) = open_dashboard(false).await?;
    let snapshot = wait_for_text(&page, "Fagbrev").await?;
    let result = print_json(&snapshot);
    finish(browser, handler_task, owns_browser).await?;
    result
}

pub fn logout() -> Result<()> {
    let profile = profile_dir()?;
    if !profile.exists() {
        println!("No local browser profile exists.");
        return Ok(());
    }

    print!(
        "Remove the local Fagbrev browser profile at {}? [y/N] ",
        profile.display()
    );
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if answer.trim().eq_ignore_ascii_case("y") {
        std::fs::remove_dir_all(&profile)?;
        println!("Local browser profile removed.");
    } else {
        println!("Kept the local browser profile.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        DocumentationDetailSnapshot, DocumentationRowSnapshot, DocumentationTableReadiness,
        FeedbackSnapshot, HalfYearTaskSnapshot, PageSnapshot, dashboard_overview_from_snapshot,
        documentation_record_from_detail, documentation_status_from_ui,
        documentation_table_is_rendered, parse_documentation_rows,
        parse_expanded_goal_from_page_text, parse_feedback, parse_goals_from_page_text,
        parse_half_year_tasks, plain_text_as_safe_html, validate_documentation_target,
    };
    use crate::data::{CompetencyGoalReference, Delmal, DocumentationStatus, DocumentationTarget};

    #[test]
    fn parses_dashboard_progress_and_counts() {
        let overview = dashboard_overview_from_snapshot(PageSnapshot {
            title: "Fagbrev - Oversikt".to_string(),
            url: "https://fagbrev.io/l".to_string(),
            text: "Velkommen. Du har fullført 38% av læretiden din, skrevet 15 dokumentasjoner og hatt 4 vurderingssamtaler. 1 Dokumentasjoner som må rettes 0 Dokumentasjoner til vurdering 14 Godkjente dokumentasjoner".to_string(),
        });

        assert!(overview.authenticated);
        assert_eq!(overview.progress_percent, Some(38));
        assert_eq!(overview.documentations_written, Some(15));
        assert_eq!(overview.assessment_conversations, Some(4));
        assert_eq!(overview.documentations_needing_correction, Some(1));
        assert_eq!(overview.documentations_to_review, Some(0));
        assert_eq!(overview.approved_documentations, Some(14));
    }

    #[test]
    fn parses_goals_from_accessible_page_text() {
        let goals = parse_goals_from_page_text(
            "1\n1. Planlegge, utvikle og dokumentere løsninger\n0\n4. Planlegge systemer for datainnsamling",
        );

        assert_eq!(goals.len(), 2);
        assert_eq!(goals[0].number, 1);
        assert_eq!(goals[1].status_count, 0);
    }

    #[test]
    fn parses_expanded_goal_bullets_and_joins_wrapped_text() {
        let (title, status_count, delmal) = parse_expanded_goal_from_page_text(
            "1\n1. Parent goal\nPlanlegging:\n * First activity continued\n   text\n * Second activity\nUtvikling:\n * Third activity\nGjennom hele prosessen kan lærlingen bruke interne rutiner.\nSe tilknyttede dokumentasjoner (1)\n2\n2. Next goal",
            1,
        );

        assert_eq!(title.as_deref(), Some("Parent goal"));
        assert_eq!(status_count, Some(1));
        assert_eq!(delmal.len(), 3);
        assert_eq!(delmal[0].number, Some(1));
        assert_eq!(delmal[0].title, "First activity continued text");
        assert_eq!(delmal[2].goal_number, 1);
    }

    #[test]
    fn parses_documentation_rows_with_real_ids_and_ui_statuses() {
        let rows = parse_documentation_rows(vec![
            DocumentationRowSnapshot {
                id: Some("realApprovedId".to_string()),
                url: Some("/l/dokumentasjon/realApprovedId".to_string()),
                title: Some("Approved work".to_string()),
                updated_at: Some("03.06.2026 10:00".to_string()),
                status_icon: Some("check-circle".to_string()),
                status_color: Some("bg-positive".to_string()),
            },
            DocumentationRowSnapshot {
                id: Some("realCorrectionId".to_string()),
                url: Some("/l/dokumentasjon/realCorrectionId".to_string()),
                title: Some("Needs correction".to_string()),
                updated_at: Some("23.06.2026 13:17".to_string()),
                status_icon: Some("edit-2".to_string()),
                status_color: Some("bg-negative".to_string()),
            },
            DocumentationRowSnapshot {
                id: Some("realDraftId".to_string()),
                url: Some("/l/dokumentasjon/realDraftId".to_string()),
                title: Some("Draft".to_string()),
                updated_at: Some("11.09.2026 10:24".to_string()),
                status_icon: Some("edit".to_string()),
                status_color: Some("bg-dark".to_string()),
            },
        ]);

        assert_eq!(rows[0].id.as_deref(), Some("realApprovedId"));
        assert_eq!(rows[0].status, DocumentationStatus::Approved);
        assert_eq!(rows[1].status, DocumentationStatus::NeedsCorrection);
        assert_eq!(rows[2].status, DocumentationStatus::Draft);
        assert_eq!(
            documentation_status_from_ui(Some("unknown"), None, None),
            DocumentationStatus::Unknown
        );
    }

    #[test]
    fn accepts_a_rendered_empty_documentation_page() {
        let readiness = DocumentationTableReadiness {
            has_table: true,
            linked_rows: 0,
            body_text: "Søk\nViser\n0\nav 0".to_string(),
        };

        assert!(documentation_table_is_rendered(&readiness));
        assert!(!documentation_table_is_rendered(
            &DocumentationTableReadiness {
                has_table: true,
                linked_rows: 0,
                body_text: "Søk\nLaster inn".to_string(),
            }
        ));
    }

    #[test]
    fn parses_documentation_detail_content_attachments_and_targets() {
        let record = documentation_record_from_detail(
            "realDocumentId".to_string(),
            "https://fagbrev.io/l/dokumentasjon/realDocumentId".to_string(),
            DocumentationDetailSnapshot {
                title: Some("My documentation".to_string()),
                status_label: Some("Godkjent".to_string()),
                status_icon: Some("check-circle".to_string()),
                status_color: Some("bg-positive".to_string()),
                updated_at: Some("03.06.2026".to_string()),
                content_text: Some("Visible documentation text".to_string()),
                content_html: Some("<p>Visible documentation text</p>".to_string()),
                attachments: vec![crate::data::DocumentationAttachment {
                    name: Some("evidence.docx".to_string()),
                    url: None,
                    mime_type: None,
                }],
                target_text: Some(
                    "LÆREPLAN\nLæreplan i IT-utviklerfaget\nKompetansemål og vurdering vg3 IT-utviklerfaget\n1 mål og 1 delmål\n15. Feilsøke kode\nDokumentere virksomhetens rutiner for feilsøking\n1 dok.".to_string(),
                ),
                feedback: vec![],
            },
        );

        assert_eq!(record.id.as_deref(), Some("realDocumentId"));
        assert_eq!(record.status, DocumentationStatus::Approved);
        assert_eq!(
            record.content.as_deref(),
            Some("Visible documentation text")
        );
        assert_eq!(
            record.content_html.as_deref(),
            Some("<p>Visible documentation text</p>")
        );
        assert_eq!(record.attachments[0].name.as_deref(), Some("evidence.docx"));
        let target = record.target_summary.expect("target summary");
        assert_eq!(target.competency_goals[0].number, 15);
        assert_eq!(target.delmal[0].id, None);
        assert_eq!(
            target.delmal[0].title,
            "Dokumentere virksomhetens rutiner for feilsøking"
        );
    }

    #[test]
    fn parses_half_year_task_fixture_with_ui_ordinals_and_counts() {
        let tasks = parse_half_year_tasks(vec![
            HalfYearTaskSnapshot {
                ordinal: 1,
                title: "Etikk, lovverk og yrkesutøvelse".to_string(),
                task_text: Some("Planlegg, gjennomfør, dokumenter og evaluer.".to_string()),
                status_label: None,
                status_icon: None,
                status_color: None,
                documentation_count: Some(2),
                linked_documentation: vec![crate::data::LinkedDocumentationSummary {
                    id: Some("doc-1".to_string()),
                    title: Some("Etikk i praksis".to_string()),
                    status: Some(DocumentationStatus::Approved),
                }],
            },
            HalfYearTaskSnapshot {
                ordinal: 6,
                title: "Minifagprøve".to_string(),
                task_text: None,
                status_label: Some("Godkjent".to_string()),
                status_icon: Some("check-circle".to_string()),
                status_color: Some("bg-positive".to_string()),
                documentation_count: Some(1),
                linked_documentation: vec![],
            },
        ]);

        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].ordinal, 1);
        assert_eq!(tasks[0].documentation_count, Some(2));
        assert_eq!(
            tasks[0].linked_documentation[0].id.as_deref(),
            Some("doc-1")
        );
        assert_eq!(tasks[0].status, None);
        assert_eq!(tasks[1].status, Some(DocumentationStatus::Approved));
        assert_eq!(tasks[1].task_text, None);
    }

    #[test]
    fn parses_feedback_fixture_without_avatar_or_other_private_data() {
        let feedback = parse_feedback(
            "realDocumentId",
            vec![FeedbackSnapshot {
                status_label: Some("Godkjent".to_string()),
                status_icon: Some("check-circle".to_string()),
                status_color: Some("bg-positive".to_string()),
                author: Some("Rune Alexander Laursen".to_string()),
                timestamp: Some("20.06.2026 11:08".to_string()),
                text: "Bra dokumentasjon.".to_string(),
            }],
        );

        assert_eq!(feedback.len(), 1);
        assert_eq!(feedback[0].ordinal, 1);
        assert_eq!(feedback[0].document_id, "realDocumentId");
        assert_eq!(feedback[0].status, Some(DocumentationStatus::Approved));
        assert_eq!(
            feedback[0].author.as_deref(),
            Some("Rune Alexander Laursen")
        );
        assert_eq!(feedback[0].timestamp.as_deref(), Some("20.06.2026 11:08"));
        assert_eq!(feedback[0].text, "Bra dokumentasjon.");
    }

    #[test]
    fn validates_write_targets_without_browser_access() {
        let valid = DocumentationTarget {
            competency_goal: Some(CompetencyGoalReference {
                number: 15,
                title: None,
                id: None,
            }),
            delmal: Some(Delmal {
                goal_number: 15,
                number: Some(1),
                title: "Dokumentere rutiner".to_string(),
                id: None,
                status: None,
                status_count: None,
            }),
        };
        assert!(validate_documentation_target(&valid).is_ok());

        let mismatched = DocumentationTarget {
            delmal: Some(Delmal {
                goal_number: 14,
                number: Some(1),
                title: "Dokumentere rutiner".to_string(),
                ..valid.delmal.clone().expect("delmål")
            }),
            ..valid.clone()
        };
        assert!(validate_documentation_target(&mismatched).is_err());

        let missing_ordinal = DocumentationTarget {
            delmal: Some(Delmal {
                number: None,
                ..valid.delmal.expect("delmål")
            }),
            competency_goal: None,
        };
        assert!(validate_documentation_target(&missing_ordinal).is_err());
    }

    #[test]
    fn escapes_plain_text_before_rich_editor_insertion() {
        let html = plain_text_as_safe_html("<script>alert('x')</script>\nnext");
        assert_eq!(
            html,
            "<p>&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;<br>next</p>"
        );
    }

    #[tokio::test]
    async fn browser_write_methods_refuse_without_confirmation() {
        let target = DocumentationTarget::default();
        let submit_error = super::submit_documentation("title", "content", &target, false)
            .await
            .expect_err("unconfirmed submit must stop before browser access");
        assert!(submit_error.to_string().contains("confirm=true"));

        let approval_error = super::request_approval("document-123", false)
            .await
            .expect_err("unconfirmed approval must stop before browser access");
        assert!(approval_error.to_string().contains("confirm=true"));
    }
}
