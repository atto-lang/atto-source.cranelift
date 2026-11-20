//! Bounded, explicitly-authorized source graph for the trusted scalar JIT.
//! This is NOT a general Atto package resolver or a native tenant sandbox.
//! No fallback to ambient ORES_PATHS, user directories, or arbitrary imports.
use crate::frontend::Program;
use crate::functional_pipelines;
use crate::open_source_file;
use std::collections::HashSet;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const MAX_IMPORTS: usize = 8;
const MAX_MODULE_BYTES: u64 = 16 * 1024;
const MAX_MANIFEST_BYTES: u64 = 4096;
const MAX_SPECIFIER_BYTES: usize = 256;

struct NamedImport {
    exported_name: String,
    local_name: String,
    specifier: String,
}

fn valid_identifier(text: &str) -> bool {
    let bytes = text.as_bytes();
    !bytes.is_empty()
        && (bytes[0].is_ascii_alphabetic() || bytes[0] == b'_')
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

fn parse_import_line(line: &str) -> Result<NamedImport, String> {
    let line = line.trim();
    let remainder = line
        .strip_prefix("import ")
        .ok_or("E_LINK_IMPORT_SYNTAX")?;
    let (name, path) = remainder
        .split_once(" from ")
        .ok_or("E_LINK_IMPORT_SYNTAX")?;
    let (exported_name, local_name) = name.split_once(" as ").unwrap_or((name, name));
    if !valid_identifier(exported_name) || !valid_identifier(local_name) {
        return Err("E_LINK_IMPORT_NAME".into());
    }
    let raw = path
        .strip_prefix('"')
        .and_then(|p| p.strip_suffix("\";"))
        .ok_or("E_LINK_IMPORT_SYNTAX")?;
    if raw.contains('"') || raw.len() > MAX_SPECIFIER_BYTES {
        return Err("E_LINK_PATH_SYNTAX".into());
    }
    Ok(NamedImport {
        exported_name: exported_name.to_owned(),
        local_name: local_name.to_owned(),
        specifier: raw.to_owned(),
    })
}

fn split_entry(source: &str) -> Result<(String, Vec<NamedImport>), String> {
    let mut stripped = String::with_capacity(source.len());
    let mut imports = Vec::new();
    let mut declarations_started = false;
    let mut unique = HashSet::new();
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("import ") {
            if declarations_started {
                return Err("E_LINK_IMPORT_ORDER".into());
            }
            if imports.len() >= MAX_IMPORTS {
                return Err("E_LINK_IMPORT_LIMIT".into());
            }
            let item = parse_import_line(trimmed)?;
            if !unique.insert(item.local_name.clone()) {
                return Err("E_LINK_DUP_BINDING".into());
            }
            imports.push(item);
            // A removed import must not merge two tokens across line boundaries.
            stripped.push('\n');
        } else {
            if !trimmed.is_empty() && !trimmed.starts_with("//") {
                declarations_started = true;
            }
            stripped.push_str(line);
        }
    }
    if imports.is_empty() {
        return Err("E_LINK_IMPORT_REQUIRED".into());
    }
    Ok((stripped, imports))
}

fn checked_path_suffix(text: &str) -> Result<&str, String> {
    if text.len() > MAX_SPECIFIER_BYTES
        || text.is_empty()
        || !text.ends_with(".atto")
        || text.contains('\\')
        || text.contains(':')
        || text.chars().any(char::is_control)
    {
        return Err("E_LINK_PATH_SYNTAX".into());
    }
    for segment in text.split('/') {
        if segment.is_empty()
            || matches!(segment, "." | "..")
            || segment.starts_with('.')
            || !segment
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        {
            return Err("E_LINK_PATH_TRAVERSAL".into());
        }
    }
    Ok(text)
}

fn load_bounded(path: &Path, max: u64) -> Result<String, String> {
    // Reuse the CLI's descriptor-relative openat/O_NOFOLLOW policy on Unix.
    let source = open_source_file(path).map_err(|reason| format!("E_LINK_SOURCE_OPEN {reason}"))?;
    let mut bytes = Vec::new();
    source
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("E_LINK_SOURCE_READ {e}"))?;
    if bytes.len() as u64 > max {
        return Err("E_LINK_SOURCE_BUDGET".into());
    }
    String::from_utf8(bytes).map_err(|_| "E_LINK_INVALID_UTF8".into())
}

fn manifest_has_package_name(source: &str) -> bool {
    // Only the minimal explicitly supported package marker format is
    // admissible. Unknown TOML keys/sections must not silently grant roots.
    let mut in_package = false;
    let mut saw_package = false;
    let mut saw_schema = false;
    let mut saw_name = false;
    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[package]" && !saw_package && saw_schema {
            saw_package = true;
            in_package = true;
            continue;
        }
        if !in_package {
            if line == r#"schema_version = "1""# && !saw_schema {
                saw_schema = true;
                continue;
            }
            return false;
        }
        if saw_name {
            return false;
        }
        let Some(value) = line.strip_prefix("name") else {
            return false;
        };
        let Some(value) = value.trim_start().strip_prefix('=') else {
            return false;
        };
        let value = value.trim();
        let Some(inner) = value.strip_prefix('"').and_then(|s| s.strip_suffix('"')) else {
            return false;
        };
        if inner.is_empty()
            || !inner
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return false;
        }
        saw_name = true;
    }
    saw_schema && saw_package && saw_name
}

fn absolute_entry(path: &Path) -> Result<PathBuf, String> {
    // Do not normalize a lexical .. that the CLI's no-follow policy rejected.
    if path.components().any(|c| matches!(c, Component::ParentDir | Component::Prefix(_))) {
        return Err("E_LINK_ENTRY_TRAVERSAL".into());
    }
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        let cwd = std::env::current_dir().map_err(|e| format!("E_LINK_CWD {e}"))?;
        Ok(cwd.join(path))
    }
}

fn package_root(entry: &Path) -> Result<PathBuf, String> {
    let mut directory = entry.parent().ok_or("E_LINK_ENTRY_DIRECTORY")?.to_path_buf();
    loop {
        let marker = directory.join(".ores.pkg.toml");
        match std::fs::symlink_metadata(&marker) {
            Ok(_) => {
                let contents = load_bounded(&marker, MAX_MANIFEST_BYTES)?;
                if !manifest_has_package_name(&contents) {
                    return Err("E_LINK_INVALID_MANIFEST".into());
                }
                return Ok(directory);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("E_LINK_MANIFEST_LOOKUP {e}")),
        }
        if !directory.pop() {
            return Err("E_LINK_PACKAGE_ROOT_REQUIRED".into());
        }
    }
}

fn require_no_nested_manifest(root: &Path, target: &Path) -> Result<(), String> {
    let relative = target
        .strip_prefix(root)
        .map_err(|_| "E_LINK_OUTSIDE_PACKAGE")?;
    let mut cursor = root.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(segment) = components.next() {
        if components.peek().is_none() {
            break; // Last component is the imported source file itself.
        }
        cursor.push(segment);
        for marker in [".ores.pkg.toml", ".oreslangc.cfg.toml"] {
            match std::fs::symlink_metadata(cursor.join(marker)) {
                Ok(_) => return Err("E_LINK_NESTED_PACKAGE".into()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("E_LINK_PACKAGE_BOUNDARY {e}")),
            }
        }
    }
    Ok(())
}

fn resolve_import(entry: &Path, root: &Path, spec: &str) -> Result<PathBuf, String> {
    let (base, remainder) = if let Some(path) = spec.strip_prefix('+') {
        // The modern spelling is +path. +/path belongs only to historical corpora.
        if path.starts_with('/') {
            return Err("E_LINK_RETIRED_ROOT_PREFIX".into());
        }
        (root, path)
    } else if let Some(path) = spec.strip_prefix("./") {
        (entry.parent().ok_or("E_LINK_ENTRY_DIRECTORY")?, path)
    } else {
        return Err("E_LINK_UNSUPPORTED_IMPORT_KIND".into());
    };
    let suffix = checked_path_suffix(remainder)?;
    let candidate = base.join(suffix);
    if !candidate.starts_with(root) {
        return Err("E_LINK_OUTSIDE_PACKAGE".into());
    }
    require_no_nested_manifest(root, &candidate)?;
    Ok(candidate)
}

/// Link only explicitly named, public scalar helpers in a manifest-backed
/// single-package source graph. The helpers are lowered by the existing
/// checked scalar inliner to *real native Cranelift code*. General native
/// function ABI calls (including non-linear arguments) are not implemented.
pub fn parse(entry_path: &Path, source: &str) -> Result<Program, String> {
    let (body, imports) = split_entry(source)?;
    let entry = absolute_entry(entry_path)?;
    let root = package_root(&entry)?;
    if !entry.starts_with(&root) {
        return Err("E_LINK_ENTRY_OUTSIDE_PACKAGE".into());
    }
    let mut units = Vec::with_capacity(imports.len());
    for import in imports {
        let path = resolve_import(&entry, &root, &import.specifier)?;
        let text = load_bounded(&path, MAX_MODULE_BYTES)?;
        units.push((import.exported_name, import.local_name, text));
    }
    functional_pipelines::parse_linked(&body, &units)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_invalid_path_spellings_fail_closed() {
        for path in ["+/src/lib/helpers.atto", "+src/../lib/helpers.atto",
                     "+src//lib/helpers.atto", "+src\\lib\\helpers.atto",
                     "+src/lib/helpers.ores", "+src/lib/./helpers.atto"] {
            let result = parse_import_line(&format!("import helper from \"{path}\";"))
                .and_then(|import| {
                    if let Some(suffix) = import.specifier.strip_prefix('+') {
                        checked_path_suffix(suffix).map(|_| ())
                    } else {
                        Err("E_LINK_UNSUPPORTED_IMPORT_KIND".into())
                    }
                });
            assert!(result.is_err(), "admitted invalid path {path}");
        }
    }
    #[test]
    fn duplicate_import_names_and_missing_imports_are_rejected() {
        assert!(split_entry("fnc main(): int { return 42; }").is_err());
        assert!(split_entry("import a from \"./lib/a.atto\";\n\
            import a from \"./lib/b.atto\";\n\
            fnc main(): int { return 42; }").is_err());
    }
}
