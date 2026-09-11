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

use crate::data::{CompetencyGoal, CompetencyGoalDetails, DashboardOverview};

const FAGBREV_URL: &str = "https://fagbrev.io/l";
const PLAN_URL: &str = "https://fagbrev.io/l/laereplanmal";
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
            });
        }
    }

    goals
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
            "() => {{ const button = Array.from(document.querySelectorAll('button')).find(button => /^{}\\.\\s/.test((button.textContent || '').replace(/\\s+/g, ' ').trim())); if (!button) return false; button.click(); return true; }}",
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
    let result = CompetencyGoalDetails {
        number,
        url: snapshot.url,
        page_title: snapshot.title,
        visible_text: snapshot.text,
    };
    finish(browser, handler_task, owns_browser).await?;
    Ok(result)
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
    use super::{PageSnapshot, dashboard_overview_from_snapshot, parse_goals_from_page_text};

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
}
