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

use crate::data::{CompetencyGoal, CompetencyGoalDetails, DashboardOverview, Delmal};

const FAGBREV_URL: &str = "https://fagbrev.io/l";
const PLAN_URL: &str = "https://fagbrev.io/l/laereplanmal";
const DOCUMENTATION_URL: &str = "https://fagbrev.io/l/dokumentasjon";
const LOGIN_WAIT: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PageSnapshot {
    title: String,
    url: String,
    text: String,
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

fn parse_picker_delmal_from_page_text(text: &str) -> Vec<Delmal> {
    let mut result = Vec::new();
    let mut goal_number = None;
    let mut next_number = 0u16;

    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some((number, _title)) = numbered_heading(line)
            && (1..=21).contains(&number)
        {
            goal_number = Some(number);
            next_number = 0;
            continue;
        }

        let Some(goal_number) = goal_number else {
            continue;
        };

        if line.contains("Ditt utvalg") || line.starts_with("Knytt denne dokumentasjonen") {
            break;
        }

        if line.starts_with("Kompetansemål og vurdering")
            || line.starts_with("KOMPETANSEMÅL OG VURDERING")
            || line.starts_with("Vis ")
            || line.starts_with("av ")
            || line.parse::<u32>().is_ok()
        {
            continue;
        }

        if line.starts_with("Halvårsoppgaver") || line.starts_with("HALVÅRSOPPGAVER") {
            break;
        }

        next_number += 1;
        result.push(Delmal {
            goal_number,
            number: Some(next_number),
            title: line.to_string(),
            id: None,
            status: None,
            status_count: None,
        });
    }

    result
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

    let (browser, handler_task, page, owns_browser) = documentation_picker_page(false).await?;
    let result = parse_picker_delmal_from_page_text(&page_snapshot(&page).await?.text);
    finish(browser, handler_task, owns_browser).await?;

    Ok(result
        .into_iter()
        .filter(|delmal| goal_number.is_none_or(|number| delmal.goal_number == number))
        .collect())
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
        PageSnapshot, dashboard_overview_from_snapshot, parse_expanded_goal_from_page_text,
        parse_goals_from_page_text, parse_picker_delmal_from_page_text,
    };

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
    fn parses_picker_delmal_with_parent_and_ordinals_without_ids() {
        let delmal = parse_picker_delmal_from_page_text(
            "Kompetansemål og vurdering vg3 IT-utviklerfaget\n0\nav 128 valgbare\n1. Parent goal\n2\nav 2 delmål\nFirst activity\nSecond activity\n2. Another goal\n0\nav 1 delmål\nOnly activity\nHalvårsoppgaver og minifagprøve",
        );

        assert_eq!(delmal.len(), 3);
        assert_eq!(delmal[0].goal_number, 1);
        assert_eq!(delmal[0].number, Some(1));
        assert_eq!(delmal[1].number, Some(2));
        assert_eq!(delmal[2].goal_number, 2);
        assert_eq!(delmal[2].id, None);
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
}
