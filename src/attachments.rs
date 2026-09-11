use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};

pub const MAX_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;

pub fn validate_output_path(output_path: &str, overwrite: bool) -> Result<PathBuf> {
    if output_path.trim().is_empty() {
        bail!("output_path is required");
    }
    let path = Path::new(output_path);
    if !path.is_absolute() {
        bail!("output_path must be an absolute path");
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        bail!("output_path must not contain '.' or '..' path components");
    }
    if path.file_name().is_none() {
        bail!("output_path must name a file");
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("output_path has no parent directory")?;
    if !parent.is_dir() {
        bail!("output_path parent directory does not exist or is not a directory");
    }
    if path.is_symlink() {
        bail!("refusing to write through a symbolic-link output path");
    }
    if path.exists() && !overwrite {
        bail!("output_path already exists; set overwrite=true to replace it");
    }
    Ok(path.to_path_buf())
}

pub fn finalize_download(temp_path: &Path, output_path: &Path, overwrite: bool) -> Result<u64> {
    let bytes = fs::metadata(temp_path)
        .with_context(|| format!("could not inspect downloaded file {}", temp_path.display()))?
        .len();
    if bytes > MAX_ATTACHMENT_BYTES {
        bail!("attachment exceeds the {MAX_ATTACHMENT_BYTES} byte download limit");
    }
    if overwrite {
        fs::rename(temp_path, output_path).with_context(|| {
            format!(
                "could not replace requested output path {}",
                output_path.display()
            )
        })?;
    } else {
        fs::hard_link(temp_path, output_path).with_context(|| {
            format!(
                "could not create requested output path {} without overwriting",
                output_path.display()
            )
        })?;
        fs::remove_file(temp_path).context("could not clean up temporary download")?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::{finalize_download, validate_output_path};
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_dir() -> std::path::PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("fagbrev-attachments-{suffix}"));
        fs::create_dir(&path).expect("create fixture directory");
        path
    }

    #[test]
    fn rejects_relative_traversal_and_missing_parents() {
        assert!(validate_output_path("attachments/file.pdf", false).is_err());
        assert!(validate_output_path("/tmp/../file.pdf", false).is_err());
        assert!(validate_output_path("/tmp/fagbrev-missing-parent/file.pdf", false).is_err());
    }

    #[test]
    fn refuses_existing_file_without_explicit_overwrite() {
        let dir = temp_dir();
        let path = dir.join("existing.txt");
        fs::write(&path, b"old").expect("write fixture");
        assert!(validate_output_path(path.to_str().expect("utf8 path"), false).is_err());
        assert!(validate_output_path(path.to_str().expect("utf8 path"), true).is_ok());
        fs::remove_dir_all(dir).expect("remove fixture directory");
    }

    #[test]
    fn finalizes_without_overwriting_existing_target() {
        let dir = temp_dir();
        let temp = dir.join("download");
        let target = dir.join("attachment.bin");
        fs::write(&temp, b"new").expect("write temp");
        assert_eq!(
            finalize_download(&temp, &target, false).expect("finalize"),
            3
        );
        assert_eq!(fs::read(&target).expect("read target"), b"new");
        assert!(!temp.exists());
        fs::remove_dir_all(dir).expect("remove fixture directory");
    }
}
