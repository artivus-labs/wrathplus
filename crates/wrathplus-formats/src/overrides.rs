//! Explicit loose-pack policy. Default archive opens never discover or enable packs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};

#[derive(Clone, Debug)]
pub(crate) struct LooseEntry {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    /// A built-in pack's file: the pack's name and the bytes, with no file on disk.
    pub builtin: Option<(&'static str, Arc<[u8]>)>,
}

/// A pack compiled into the client: named files served from memory, mounted below every loose
/// pack and in every profile ([`AssetOverrides::with_builtin_pack`]).
#[derive(Clone, Debug)]
pub struct BuiltinPack {
    name: &'static str,
    files: Vec<(String, Arc<[u8]>)>,
}

impl BuiltinPack {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            files: Vec::new(),
        }
    }

    /// Add a file at an archive path (`/` or `\`, any case).
    pub fn with_file(mut self, path: &str, bytes: Arc<[u8]>) -> Self {
        self.files.push((path.replace('\\', "/"), bytes));
        self
    }
}

/// Ordered loose roots mirroring MPQ paths; later roots win. Snapshot names at boot, read bytes
/// when requested. Symlinks and ambiguous case-folded names are rejected, not followed.
#[derive(Clone, Debug, Default)]
pub struct AssetOverrides {
    pub(crate) entries: BTreeMap<String, LooseEntry>,
}

pub(crate) fn key(name: &str) -> Option<String> {
    let name = name.replace('\\', "/");
    if name.is_empty()
        || name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || part.contains(':'))
        || name.contains('\0')
    {
        return None;
    }
    Some(name.to_ascii_lowercase())
}

impl AssetOverrides {
    pub fn from_roots(roots: impl IntoIterator<Item = PathBuf>) -> Result<Self> {
        let mut policy = Self::default();
        for root in roots {
            let mut pack = BTreeMap::new();
            collect(&root, &root, &mut pack)?;
            policy.entries.extend(pack);
        }
        Ok(policy)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Mount `pack` below every loose pack: it serves its files ahead of the archives whatever
    /// the profile or pack settings, and an enabled pack's file of the same path still wins.
    pub fn with_builtin_pack(mut self, pack: BuiltinPack) -> Self {
        for (name, bytes) in pack.files {
            let Some(normalized) = key(&name) else {
                continue;
            };
            self.entries.entry(normalized).or_insert(LooseEntry {
                path: PathBuf::new(),
                size: bytes.len() as u64,
                builtin: Some((pack.name, bytes)),
                name,
            });
        }
        self
    }

    pub(crate) fn resolve(&self, name: &str) -> Option<&LooseEntry> {
        self.entries.get(&key(name)?)
    }
}

fn collect(root: &Path, dir: &Path, out: &mut BTreeMap<String, LooseEntry>) -> Result<()> {
    let metadata = std::fs::symlink_metadata(dir)
        .with_context(|| format!("opening asset pack {}", dir.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "asset pack directory must not be a symlink: {}",
            dir.display()
        );
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            bail!("asset pack symlinks are forbidden: {}", path.display());
        }
        if metadata.is_dir() {
            collect(root, &path, out)?;
        } else if metadata.is_file() {
            let name = path
                .strip_prefix(root)?
                .to_str()
                .context("non-UTF8 asset pack path")?
                .replace('\\', "/");
            let normalized = key(&name).context("unsafe asset pack path")?;
            let value = LooseEntry {
                name,
                path,
                size: metadata.len(),
                builtin: None,
            };
            if out.insert(normalized, value).is_some() {
                bail!("ambiguous case-folded names in pack {}", root.display());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn pack_symlinks_are_rejected_at_mount() {
        let root =
            std::env::temp_dir().join(format!("wrathplus-pack-symlink-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink("/etc", root.join("outside")).unwrap();
        assert!(AssetOverrides::from_roots([root.clone()]).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn enabled_policy_preserves_archive_fallback_and_default_opens() {
        let data = crate::wow_data_or_skip!();
        let root =
            std::env::temp_dir().join(format!("wrathplus-pack-fallback-{}", std::process::id()));
        std::fs::create_dir_all(root.join("DBFilesClient")).unwrap();
        std::fs::write(root.join("DBFilesClient/Spell.dbc"), b"pack-table").unwrap();
        let original = crate::open_configured_chain(&data).unwrap();
        let name = "DBFilesClient/Spell.dbc";
        assert_eq!(&original.read(name).unwrap()[..4], b"WDBC");
        let fallback = "DBFilesClient/Map.dbc";
        let expected = original.read(fallback).unwrap();
        let policy = AssetOverrides::from_roots([root.clone()]).unwrap();
        let overridden = original.with_overrides(policy);
        assert_eq!(overridden.read(name).unwrap(), b"pack-table");
        assert_eq!(overridden.read(fallback).unwrap(), expected);
        assert_eq!(
            &crate::open_configured_chain(&data)
                .unwrap()
                .read(name)
                .unwrap()[..4],
            b"WDBC"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn packs_share_read_contains_and_enumeration_with_last_pack_winning() {
        let root =
            std::env::temp_dir().join(format!("wrathplus-pack-policy-{}", std::process::id()));
        let a = root.join("a");
        let b = root.join("b");
        for dir in [&a, &b] {
            std::fs::create_dir_all(dir.join("World")).unwrap();
        }
        std::fs::write(a.join("World/Tile.blp"), b"original").unwrap();
        std::fs::write(b.join("World/Tile.blp"), b"replacement").unwrap();
        std::fs::write(a.join("Model.m2"), b"model").unwrap();
        std::fs::write(b.join("Table.dbc"), b"dbc").unwrap();
        let policy = AssetOverrides::from_roots([a, b]).unwrap();
        let chain = crate::Chain::from_overrides(policy, crate::DataProfile::default());
        assert!(chain.contains("WORLD\\TILE.BLP"));
        assert_eq!(chain.read("world/tile.blp").unwrap(), b"replacement");
        assert_eq!(chain.read("model.m2").unwrap(), b"model");
        assert_eq!(chain.read("table.dbc").unwrap(), b"dbc");
        let list = chain
            .list_filtered(|p| p.to_ascii_lowercase().ends_with(".blp"))
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].size, 11);
        assert!(!chain.contains("missing.blp"));
        for unsafe_path in [
            "../Model.m2",
            "/Model.m2",
            "World/../Model.m2",
            "C:Model.m2",
        ] {
            assert!(!chain.contains(unsafe_path));
            assert!(chain.read(unsafe_path).is_err());
        }
        let classic =
            crate::Chain::from_overrides(Default::default(), crate::DataProfile::default());
        assert!(!classic.contains("model.m2"));
        assert!(classic.list().unwrap().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
