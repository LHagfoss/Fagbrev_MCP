//! Persistent local-only documentation drafts.
//!
//! Draft files live in the OS application-data directory, next to (but
//! separate from) the browser profile. They contain only user-authored draft
//! data and references; authentication state is never read or written here.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use directories::ProjectDirs;

use crate::{browser, data};

const DRAFTS_DIRECTORY: &str = "drafts";
const MAX_DRAFTS: u32 = 100;
const MAX_TITLE_CHARS: usize = 500;
const MAX_CONTENT_CHARS: usize = 100_000;
const MAX_SOURCE_IDS: usize = 100;
const MAX_SOURCE_ID_CHARS: usize = 256;

#[derive(Debug, Clone)]
pub struct DraftStore {
    directory: PathBuf,
}

impl DraftStore {
    pub fn from_app_data() -> Result<Self> {
        let project_dirs = ProjectDirs::from("io", "fagbrev", "fagbrev-mcp")
            .context("could not determine an OS application-data directory")?;
        Ok(Self::new(project_dirs.data_dir().join(DRAFTS_DIRECTORY)))
    }

    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn create(
        &self,
        title: String,
        content: String,
        target: data::DocumentationTarget,
        source_documentation_ids: Vec<String>,
    ) -> Result<data::LocalDraft> {
        validate_fields(&title, &content, &target, &source_documentation_ids)?;
        let timestamp = current_timestamp();
        let draft = data::LocalDraft {
            draft_id: new_draft_id(),
            title,
            content,
            target,
            source_documentation_ids,
            version: 1,
            created_at: timestamp.clone(),
            updated_at: timestamp,
            status: data::LocalDraftStatus::LocalOnly,
        };
        self.write(&draft)?;
        Ok(draft)
    }

    pub fn update(
        &self,
        draft_id: &str,
        title: String,
        content: String,
        target: data::DocumentationTarget,
        source_documentation_ids: Vec<String>,
        expected_version: u64,
    ) -> Result<data::LocalDraft> {
        validate_draft_id(draft_id)?;
        validate_fields(&title, &content, &target, &source_documentation_ids)?;
        let current = self.get(draft_id)?;
        if current.version != expected_version {
            bail!(
                "draft {draft_id} changed since it was read: expected version {expected_version}, current version {}",
                current.version
            );
        }
        let draft = data::LocalDraft {
            draft_id: current.draft_id,
            title,
            content,
            target,
            source_documentation_ids,
            version: current
                .version
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("draft version limit reached"))?,
            created_at: current.created_at,
            updated_at: current_timestamp(),
            status: data::LocalDraftStatus::LocalOnly,
        };
        self.write(&draft)?;
        Ok(draft)
    }

    pub fn get(&self, draft_id: &str) -> Result<data::LocalDraft> {
        validate_draft_id(draft_id)?;
        let path = self.path_for(draft_id);
        let bytes =
            fs::read(&path).with_context(|| format!("could not read local draft {draft_id}"))?;
        let draft: data::LocalDraft = serde_json::from_slice(&bytes)
            .with_context(|| format!("local draft {draft_id} is invalid JSON"))?;
        if draft.draft_id != draft_id {
            bail!("local draft file does not match requested draft_id");
        }
        if draft.status != data::LocalDraftStatus::LocalOnly {
            bail!("draft {draft_id} has an unsupported local status");
        }
        Ok(draft)
    }

    pub fn list(&self, limit: Option<u32>) -> Result<Vec<data::LocalDraftSummary>> {
        let limit = limit.unwrap_or(50).clamp(1, MAX_DRAFTS) as usize;
        if !self.directory.exists() {
            return Ok(Vec::new());
        }

        let mut drafts = Vec::new();
        for entry in fs::read_dir(&self.directory).with_context(|| {
            format!(
                "could not list local drafts in {}",
                self.directory.display()
            )
        })? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let draft_id = path
                .file_stem()
                .and_then(|name| name.to_str())
                .ok_or_else(|| anyhow::anyhow!("local draft filename is not valid UTF-8"))?;
            drafts.push(data::LocalDraftSummary::from(&self.get(draft_id)?));
        }
        drafts.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        drafts.truncate(limit);
        Ok(drafts)
    }

    pub fn delete(&self, draft_id: &str) -> Result<()> {
        validate_draft_id(draft_id)?;
        let path = self.path_for(draft_id);
        if !path.exists() {
            bail!("local draft {draft_id} was not found");
        }
        fs::remove_file(&path)
            .with_context(|| format!("could not delete local draft {draft_id}"))?;
        Ok(())
    }

    fn write(&self, draft: &data::LocalDraft) -> Result<()> {
        validate_draft_id(&draft.draft_id)?;
        fs::create_dir_all(&self.directory).with_context(|| {
            format!(
                "could not create local draft directory {}",
                self.directory.display()
            )
        })?;

        let bytes = serde_json::to_vec_pretty(draft)?;
        let temporary =
            self.directory
                .join(format!(".{}.{}.tmp", draft.draft_id, current_timestamp()));
        let result = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .with_context(|| {
                    format!(
                        "could not create temporary draft file {}",
                        temporary.display()
                    )
                })?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, self.path_for(&draft.draft_id)).with_context(|| {
                format!(
                    "could not atomically replace local draft {}",
                    draft.draft_id
                )
            })?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    fn path_for(&self, draft_id: &str) -> PathBuf {
        self.directory.join(format!("{draft_id}.json"))
    }
}

pub(crate) fn current_timestamp() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    nanos.to_string()
}

fn new_draft_id() -> String {
    format!("draft-{}-{}", current_timestamp(), process::id())
}

fn validate_draft_id(draft_id: &str) -> Result<()> {
    if draft_id.is_empty()
        || draft_id.len() > 128
        || !draft_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        bail!("draft_id must be a local draft identifier");
    }
    Ok(())
}

fn validate_fields(
    title: &str,
    content: &str,
    target: &data::DocumentationTarget,
    source_documentation_ids: &[String],
) -> Result<()> {
    if title.trim().is_empty() {
        bail!("title cannot be empty");
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        bail!("title cannot exceed {MAX_TITLE_CHARS} characters");
    }
    if content.trim().is_empty() {
        bail!("content cannot be empty");
    }
    if content.chars().count() > MAX_CONTENT_CHARS {
        bail!("content cannot exceed {MAX_CONTENT_CHARS} characters");
    }
    browser::validate_documentation_target(target)?;
    if source_documentation_ids.len() > MAX_SOURCE_IDS {
        bail!("source_documentation_ids cannot contain more than {MAX_SOURCE_IDS} IDs");
    }
    for document_id in source_documentation_ids {
        if document_id.is_empty()
            || document_id.chars().count() > MAX_SOURCE_ID_CHARS
            || !document_id.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
        {
            bail!("source_documentation_ids must contain Fagbrev documentation IDs");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, time::SystemTime};

    use super::DraftStore;
    use crate::data::{CompetencyGoalReference, Delmal, DocumentationTarget, LocalDraftStatus};

    fn temporary_store() -> DraftStore {
        let directory = std::env::temp_dir().join(format!(
            "fagbrev-mcp-drafts-{}",
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos()
        ));
        DraftStore::new(directory)
    }

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
                title: "Planlegge arbeidet".to_string(),
                id: None,
                status: None,
                status_count: None,
            }),
        }
    }

    #[test]
    fn creates_lists_reads_updates_and_deletes_only_local_files() {
        let store = temporary_store();
        let draft = store
            .create(
                "A local draft".to_string(),
                "Draft content".to_string(),
                target(),
                vec!["document-1".to_string()],
            )
            .expect("draft should be created");
        assert_eq!(draft.version, 1);
        assert_eq!(draft.status, LocalDraftStatus::LocalOnly);
        assert_eq!(store.list(None).expect("draft should list").len(), 1);

        let updated = store
            .update(
                &draft.draft_id,
                "Updated draft".to_string(),
                "Updated content".to_string(),
                target(),
                Vec::new(),
                1,
            )
            .expect("draft should update");
        assert_eq!(updated.version, 2);
        assert_eq!(
            store.get(&draft.draft_id).expect("draft should read").title,
            "Updated draft"
        );
        store.delete(&draft.draft_id).expect("draft should delete");
        assert!(
            store
                .list(None)
                .expect("draft list should be empty")
                .is_empty()
        );
        let _ = fs::remove_dir_all(store.directory);
    }

    #[test]
    fn rejects_stale_versions_and_path_like_ids() {
        let store = temporary_store();
        let draft = store
            .create(
                "Title".to_string(),
                "Content".to_string(),
                target(),
                Vec::new(),
            )
            .expect("draft should be created");
        let error = store
            .update(
                &draft.draft_id,
                "New".to_string(),
                "New content".to_string(),
                target(),
                Vec::new(),
                9,
            )
            .expect_err("stale version should be rejected");
        assert!(error.to_string().contains("current version"));
        let error = store
            .get("../credentials")
            .expect_err("path traversal must be rejected");
        assert!(error.to_string().contains("draft_id"));
        let _ = fs::remove_dir_all(store.directory);
    }

    #[test]
    fn rejects_unbounded_draft_content() {
        let store = temporary_store();
        let error = store
            .create(
                "Title".to_string(),
                "x".repeat(super::MAX_CONTENT_CHARS + 1),
                target(),
                Vec::new(),
            )
            .expect_err("oversized local content should be rejected");
        assert!(error.to_string().contains("cannot exceed"));
        let _ = fs::remove_dir_all(store.directory);
    }
}
