//! Read-only stock archive compatibility checks.
use super::*;

pub(super) fn find_named(directory: &Path, name: &str) -> Result<Option<PathBuf>> {
    let mut found = None;
    for entry in fs::read_dir(directory).map_err(|e| io_error(directory.display(), e))? {
        let entry = entry.map_err(|e| io_error("read directory entry", e))?;
        if entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(name)
        {
            if found.is_some() {
                return Err(format!("Ambiguous case-insensitive asset name {name}"));
            }
            found = Some(entry.path());
        }
    }
    Ok(found)
}

/// Same ascending stock mount order as formats::chain; deliberately no inherited nonstock or
/// loose-pack overrides. Later archives win, including tombstones and unreadable patch entries.
pub(super) fn open_archives(config: &Config) -> Result<Vec<Archive>> {
    let locale_dir = find_named(&config.data, &config.locale)?
        .filter(|p| p.is_dir())
        .ok_or_else(|| format!("Missing locale directory {}", config.locale))?;
    let mut paths = Vec::new();
    for name in [
        "common.MPQ",
        "common-2.MPQ",
        "expansion.MPQ",
        "lichking.MPQ",
    ] {
        if let Some(path) = find_named(&config.data, name)? {
            paths.push(path);
        }
    }
    for stem in [
        "locale",
        "speech",
        "expansion-locale",
        "expansion-speech",
        "lichking-locale",
        "lichking-speech",
    ] {
        if let Some(path) = find_named(&locale_dir, &format!("{stem}-{}.MPQ", config.locale))? {
            paths.push(path);
        }
    }
    for (dir, prefix) in [
        (&config.data, "patch".to_owned()),
        (&locale_dir, format!("patch-{}", config.locale)),
    ] {
        for suffix in ["", "-1", "-2", "-3"] {
            if let Some(path) = find_named(dir, &format!("{prefix}{suffix}.MPQ"))? {
                paths.push(path);
            }
        }
    }
    if paths.is_empty() {
        return Err("No stock Wrath MPQ archives found".into());
    }
    paths
        .iter()
        .map(|path| Archive::open(path).map_err(|e| io_error(path.display(), e)))
        .collect()
}

pub(super) fn chain_read(archives: &[Archive], name: &str, limit: u32) -> Result<Vec<u8>> {
    let archive = archives
        .iter()
        .rev()
        .find(|a| a.contains(name))
        .ok_or_else(|| format!("Missing required archive asset {name}"))?;
    if archive.is_delete_marker(name) {
        return Err(format!("Required asset {name} is deleted by a patch"));
    }
    let size = archive.file_size(name).unwrap_or(0);
    if size == 0 || size > limit {
        return Err(format!("Invalid size {size} for {name}"));
    }
    archive
        .read_file(name)
        .map_err(|e| io_error(format!("read {name} from {}", archive.path().display()), e))
}

pub(super) fn attribute<'a>(mut text: &'a str, wanted: &str) -> Option<&'a str> {
    let mut found = None;
    loop {
        text = text.trim_start();
        if text.is_empty() || text == "/" {
            return found;
        }
        let end = text.find(|c: char| c.is_whitespace() || c == '=')?;
        let name = &text[..end];
        text = text[end..].trim_start().strip_prefix('=')?.trim_start();
        let quote = text.chars().next()?;
        if quote != '\'' && quote != '"' {
            return None;
        }
        let end = text[1..].find(quote)? + 1;
        let value = &text[1..end];
        text = &text[end + 1..];
        if name == wanted {
            if found.is_some() {
                return None;
            }
            found = Some(value);
        }
    }
}

pub(super) fn check_component(bytes: &[u8], locale: &str) -> Result<()> {
    let text =
        std::str::from_utf8(bytes).map_err(|e| io_error("component manifest is not UTF-8", e))?;
    // Remove comments before looking at component tags, including the usual componentinfo
    // wrapper. Only one wow component can attest a build; unrelated XML text is not evidence.
    let mut clean = String::new();
    let mut rest = text.trim_start_matches('\u{feff}');
    while let Some((before, after)) = rest.split_once("<!--") {
        clean.push_str(before);
        rest = after
            .split_once("-->")
            .ok_or("Unclosed component manifest comment")?
            .1;
    }
    clean.push_str(rest);
    // Installed build-12340 component.wow-enUS.txt names its component wow-enus;
    // locale is encoded in the name, not in a separate locale attribute.
    let component_name = format!("wow-{}", locale.to_ascii_lowercase());
    let tags: Vec<_> = clean
        .split("<component")
        .skip(1)
        .filter(|s| s.starts_with(char::is_whitespace))
        .filter_map(|s| s.split_once('>').map(|(tag, _)| tag))
        .filter(|tag| attribute(tag, "name") == Some(component_name.as_str()))
        .collect();
    if tags.len() != 1 {
        return Err("Expected exactly one wow component manifest entry".into());
    }
    let tag = tags[0];
    if attribute(tag, "version") != Some("12340") {
        return Err(format!(
            "Assets must be Wrath build 12340 with locale {locale}"
        ));
    }
    Ok(())
}

pub(super) fn check_dbc(bytes: &[u8], name: &str) -> Result<()> {
    if bytes.len() < 20 || &bytes[..4] != b"WDBC" {
        return Err(format!("Invalid WDBC header: {name}"));
    }
    let word = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as u64;
    let (rows, fields, width, strings) = (word(4), word(8), word(12), word(16));
    if rows == 0
        || fields == 0
        || width != fields * 4
        || strings == 0
        || 20 + rows * width + strings != bytes.len() as u64
    {
        return Err(format!("Invalid WDBC layout: {name}"));
    }
    Ok(())
}

pub(crate) fn validate(config: &Config) -> Result<Validated> {
    let config = checked_config(config)?;
    let archives = open_archives(&config)?;
    let component = format!("component.wow-{}.txt", config.locale);
    check_component(
        &chain_read(&archives, &component, 64 * 1024)?,
        &config.locale,
    )?;
    for name in [
        "DBFilesClient\\ChrRaces.dbc",
        "DBFilesClient\\ChrClasses.dbc",
        "DBFilesClient\\Map.dbc",
        "DBFilesClient\\Spell.dbc",
    ] {
        check_dbc(&chain_read(&archives, name, 128 * 1024 * 1024)?, name)?;
    }
    for name in [
        "Interface\\FrameXML\\FrameXML.toc",
        "Interface\\FrameXML\\UIParent.lua",
        "Interface\\GlueXML\\GlueXML.toc",
    ] {
        let bytes = chain_read(&archives, name, 16 * 1024 * 1024)?;
        if !bytes.iter().any(|b| !b.is_ascii_whitespace()) {
            return Err(format!("Empty UI asset {name}"));
        }
    }
    Ok(Validated { summary: format!("Wrath build 12340 / {}: {} readable stock archives, essential DBC and FrameXML validated", config.locale, archives.len()), config })
}
