//! Stable content-change detector (not a cryptographic authenticity guarantee).
use super::*;
fn tree(path: &Path, hash: &mut u64, bytes: &mut u64) -> Result<()> {
    let mut entries = std::fs::read_dir(path)
        .map_err(|e| e.to_string())?
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if e.file_name() == ".git" {
            continue;
        }
        for b in e.file_name().to_string_lossy().bytes() {
            *hash = (*hash ^ u64::from(b)).wrapping_mul(1099511628211);
        }
        let meta = std::fs::symlink_metadata(e.path()).map_err(|e| e.to_string())?;
        if meta.is_dir() {
            tree(&e.path(), hash, bytes)?;
        } else if meta.is_file() {
            use std::io::Read;
            let mut file = std::fs::File::open(e.path()).map_err(|e| e.to_string())?;
            let mut buf = [0u8; 16384];
            loop {
                let n = file.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                *bytes += n as u64;
                if *bytes > 512 * 1024 * 1024 {
                    return Err("Module fingerprint exceeds 512 MiB".into());
                }
                for b in &buf[..n] {
                    *hash = (*hash ^ u64::from(*b)).wrapping_mul(1099511628211);
                }
            }
        } else {
            return Err("Module fingerprint rejects links and special files".into());
        }
        *hash = (*hash ^ 255).wrapping_mul(1099511628211);
    }
    Ok(())
}
pub(crate) fn fingerprint(path: &Path) -> Result<String> {
    let mut hash = 14695981039346656037;
    tree(path, &mut hash, &mut 0)?;
    Ok(format!("{hash:016x}"))
}
