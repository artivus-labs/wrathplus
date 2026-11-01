//! Isolated module-only Docker context; never modify the selected core or module sources.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

pub(crate) struct Staged {
    pub path: PathBuf,
}
impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
fn copy(source: &Path, dest: &Path, count: &mut usize, bytes: &mut u64) -> Result<()> {
    std::fs::create_dir(dest).map_err(|e| e.to_string())?;
    for item in std::fs::read_dir(source).map_err(|e| e.to_string())? {
        let item = item.map_err(|e| e.to_string())?;
        if item.file_name() == ".git" {
            continue;
        }
        let name = item.file_name().to_string_lossy().to_ascii_lowercase();
        if name.ends_with(".conf") || name == ".env" || name.starts_with(".env.") {
            return Err("Module source contains private working configuration. Move .conf/.env files to the managed module-config directory before building; only .conf.dist templates belong in images.".into());
        }
        let meta = std::fs::symlink_metadata(item.path()).map_err(|e| e.to_string())?;
        *count += 1;
        *bytes = bytes.saturating_add(meta.len());
        if *count > 100_000 || *bytes > 512 * 1024 * 1024 {
            return Err("Selected module context exceeds 512 MiB / 100,000 entries".into());
        }
        if meta.is_dir() {
            copy(&item.path(), &dest.join(item.file_name()), count, bytes)?;
        } else if meta.is_file() {
            std::fs::copy(item.path(), dest.join(item.file_name())).map_err(|e| e.to_string())?;
        } else {
            return Err(
                "Module context contains a symlink or special file; review the source first".into(),
            );
        }
    }
    Ok(())
}
pub(crate) fn stage(config: &Config, plan: &BuildPlan) -> Result<Staged> {
    let root = storage::root(config, &plan.core_path)?.join("build-contexts");
    if setup::resolved(&root)? != root {
        return Err("Build context root must not be a symlink".into());
    }
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let path = root.join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path).map_err(|e| format!("Reserve build context: {e}"))?;
    let staged = Staged { path };
    let (mut count, mut bytes) = (0, 0);
    for input in &plan.selected {
        copy(
            &input.source,
            &staged.path.join(&input.name),
            &mut count,
            &mut bytes,
        )?;
    }
    Ok(staged)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copy_rejects_links_and_excludes_git() {
        let root = std::env::temp_dir().join(format!("module-copy-{}", std::process::id()));
        std::fs::create_dir_all(root.join("source/.git")).unwrap();
        std::fs::write(root.join("source/code.cpp"), "source").unwrap();
        copy(&root.join("source"), &root.join("dest"), &mut 0, &mut 0).unwrap();
        assert!(!root.join("dest/.git").exists());
        assert!(root.join("dest/code.cpp").exists());
        std::fs::write(root.join("source/private.conf"), "secret").unwrap();
        assert!(copy(
            &root.join("source"),
            &root.join("private-rejected"),
            &mut 0,
            &mut 0
        )
        .unwrap_err()
        .contains("private working"));
        std::fs::remove_file(root.join("source/private.conf")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("code.cpp", root.join("source/link")).unwrap();
            assert!(copy(&root.join("source"), &root.join("rejected"), &mut 0, &mut 0).is_err());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
