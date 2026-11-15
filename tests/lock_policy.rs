//! Rust-only Cargo.lock admission tests. No Python or additional dependencies.
//! Structural package provenance is not a RustSec/security advisory scan.
use std::collections::{BTreeMap, BTreeSet};

const REGISTRY: &str = "registry+https://github.com/rust-lang/crates.io-index";
const CRANELIFT_VERSION: &str = "0.137.0";
const REAL_LOCK: &str = include_str!("../Cargo.lock");
const REAL_MANIFEST: &str = include_str!("../Cargo.toml");

#[derive(Default, Debug, Clone)]
struct Package {
    fields: BTreeMap<String, String>,
    dependencies: Vec<String>,
}

fn quoted(value: &str) -> Result<String, String> {
    let value = value
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or("expected quoted TOML string")?;
    if value.contains(['"', '\\', '\r', '\n', '\0']) {
        return Err("unsupported TOML string escape/control".into());
    }
    Ok(value.to_owned())
}

fn parse_lock(text: &str) -> Result<(Vec<Package>, u32), String> {
    let mut packages = Vec::new();
    let mut current: Option<Package> = None;
    let mut version = None;
    let mut reading_dependencies = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if reading_dependencies {
            if line == "]" {
                reading_dependencies = false;
                continue;
            }
            let item = line.strip_suffix(',').ok_or("invalid dependency array item")?;
            let name = quoted(item)?;
            current
                .as_mut()
                .ok_or("dependency without package")?
                .dependencies
                .push(name);
            continue;
        }
        if line == "[[package]]" {
            if let Some(package) = current.take() {
                packages.push(package);
            }
            current = Some(Package::default());
            continue;
        }
        if line.starts_with('[') {
            return Err("unknown lockfile table".into());
        }
        let (key, value) = line.split_once('=').ok_or("invalid lockfile field")?;
        let key = key.trim();
        let value = value.trim();
        if let Some(package) = current.as_mut() {
            match key {
                "name" | "version" | "source" | "checksum" => {
                    if package
                        .fields
                        .insert(key.into(), quoted(value)?)
                        .is_some()
                    {
                        return Err("duplicate lockfile key".into());
                    }
                }
                "dependencies" if value == "[" => {
                    if package.fields.contains_key("dependencies") {
                        return Err("duplicate dependency array".into());
                    }
                    package.fields.insert("dependencies".into(), String::new());
                    reading_dependencies = true;
                }
                _ => return Err("unapproved lockfile field".into()),
            }
        } else if key == "version" && version.is_none() {
            version = Some(value.parse::<u32>().map_err(|_| "invalid lock format version")?);
        } else {
            return Err("invalid top-level lockfile field".into());
        }
    }
    if reading_dependencies {
        return Err("unterminated dependency list".into());
    }
    if let Some(package) = current {
        packages.push(package);
    }
    Ok((packages, version.ok_or("missing lock version")?))
}

fn parse_manifest(text: &str) -> Result<(String, String, BTreeMap<String, String>), String> {
    let mut section = "";
    let mut name = None;
    let mut version = None;
    let mut direct = BTreeMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            section = match line {
                "[package]" => "package",
                "[dependencies]" => "dependencies",
                "[profile.release]" => "profile",
                _ => return Err("unapproved manifest section".into()),
            };
            continue;
        }
        let (key, value) = line.split_once('=').ok_or("invalid manifest field")?;
        let (key, value) = (key.trim(), value.trim());
        match section {
            "package" if key == "name" => {
                if name.replace(quoted(value)?).is_some() {
                    return Err("duplicate root name".into());
                }
            }
            "package" if key == "version" => {
                if version.replace(quoted(value)?).is_some() {
                    return Err("duplicate root version".into());
                }
            }
            "package" | "profile" => {}
            "dependencies" => {
                if direct.insert(key.to_owned(), quoted(value)?).is_some() {
                    return Err("duplicate direct dependency".into());
                }
            }
            _ => return Err("manifest field outside approved section".into()),
        }
    }
    Ok((
        name.ok_or("missing root name")?,
        version.ok_or("missing root version")?,
        direct,
    ))
}

fn validate(lock: &str, manifest: &str) -> Result<usize, String> {
    let (packages, version) = parse_lock(lock)?;
    if version != 4 {
        return Err("Cargo.lock format must be v4".into());
    }
    if packages.is_empty() {
        return Err("no locked packages".into());
    }
    let (root_name, root_version, direct) = parse_manifest(manifest)?;
    if direct.is_empty() {
        return Err("direct dependencies are missing".into());
    }
    let mut seen = BTreeSet::new();
    let mut locked: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut roots = 0;
    let mut root_deps: Option<&Vec<String>> = None;
    for package in &packages {
        let name = package.fields.get("name").ok_or("missing package name")?;
        let pkg_version = package.fields.get("version").ok_or("missing package version")?;
        if !seen.insert((name, pkg_version)) {
            return Err("duplicate locked package".into());
        }
        locked.entry(name).or_default().insert(pkg_version);
        if let Some(source) = package.fields.get("source") {
            if source != REGISTRY {
                return Err("non-crates.io package source".into());
            }
            let checksum = package.fields.get("checksum").ok_or("missing checksum")?;
            if checksum.len() != 64
                || !checksum.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            {
                return Err("missing/invalid SHA-256 checksum".into());
            }
            if name.starts_with("cranelift-") && pkg_version != CRANELIFT_VERSION {
                return Err("Cranelift version drift".into());
            }
        } else if name == &root_name && pkg_version == &root_version
            && !package.fields.contains_key("checksum")
        {
            roots += 1;
            root_deps = Some(&package.dependencies);
        } else {
            return Err("unapproved local/path package".into());
        }
    }
    if roots != 1 {
        return Err("exactly one root local package is required".into());
    }
    let root_deps = root_deps.ok_or("missing root dependencies")?;
    let expected: BTreeSet<&str> = direct.keys().map(String::as_str).collect();
    let actual: BTreeSet<&str> = root_deps.iter().map(String::as_str).collect();
    if actual != expected || actual.len() != root_deps.len() {
        return Err("root locked dependencies differ from Cargo.toml".into());
    }
    for (name, pin) in &direct {
        let wanted = pin.strip_prefix('=').filter(|value| !value.is_empty())
            .ok_or("direct dependency is not exactly pinned")?;
        let versions = locked.get(name.as_str()).ok_or("missing direct lock dependency")?;
        if versions.len() != 1 || !versions.contains(wanted) {
            return Err("direct dependency lock mismatch".into());
        }
    }
    Ok(packages.len())
}

const FIXTURE_MANIFEST: &str = "[package]\nname = \"orescranelift\"\nversion = \"0.1.0\"\n[dependencies]\ncranelift-codegen = \"=0.137.0\"\nlibc = \"=0.2.191\"\n";
const FIXTURE_LOCK: &str = "version = 4\n[[package]]\nname = \"orescranelift\"\nversion = \"0.1.0\"\ndependencies = [\n \"cranelift-codegen\",\n \"libc\",\n]\n[[package]]\nname = \"cranelift-codegen\"\nversion = \"0.137.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n[[package]]\nname = \"libc\"\nversion = \"0.2.191\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"\n";

#[test]
fn actual_locked_dependency_graph_matches_exact_policy() {
    assert!(validate(REAL_LOCK, REAL_MANIFEST).unwrap() > 10);
}

#[test]
fn basic_valid_registry_fixture() {
    assert_eq!(validate(FIXTURE_LOCK, FIXTURE_MANIFEST), Ok(3));
}

#[test]
fn rejects_unapproved_sources_checksums_and_cranelift_drift() {
    for (find, replacement, expected) in [
        (REGISTRY, "git+https://example.invalid/repo", "non-crates.io"),
        (REGISTRY, "registry+https://github.com/rust-lang/crates.io-index/extra", "non-crates.io"),
        ("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", "checksum"),
        ("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "checksum"),
        ("cranelift-codegen\"\nversion = \"0.137.0", "cranelift-codegen\"\nversion = \"0.136.0", "Cranelift version drift"),
    ] {
        let rejected = FIXTURE_LOCK.replacen(find, replacement, 1);
        assert!(validate(&rejected, FIXTURE_MANIFEST).unwrap_err().contains(expected));
    }
}

#[test]
fn rejects_direct_pin_drift_and_unpinned_manifest() {
    for bad in ["^0.2", "0.2.191", "*", ">=0.2.191", "=0.2.190"] {
        let altered = FIXTURE_MANIFEST.replace("libc = \"=0.2.191\"", &format!("libc = \"{bad}\""));
        let error = validate(FIXTURE_LOCK, &altered).unwrap_err();
        assert!(error.contains("exactly pinned") || error.contains("lock mismatch"), "{bad}: {error}");
    }
}

#[test]
fn rejects_duplicate_local_and_wrong_root_dependencies() {
    let duplicate = format!("{FIXTURE_LOCK}[[package]]\nname = \"orescranelift\"\nversion = \"0.1.0\"\n");
    assert!(validate(&duplicate, FIXTURE_MANIFEST).unwrap_err().contains("duplicate"));
    let shadow = format!("{FIXTURE_LOCK}[[package]]\nname = \"shadow\"\nversion = \"0.1.0\"\n");
    assert!(validate(&shadow, FIXTURE_MANIFEST).unwrap_err().contains("local/path"));
    let omitted = FIXTURE_LOCK.replace(" \"libc\",\n", "");
    assert!(validate(&omitted, FIXTURE_MANIFEST).unwrap_err().contains("root locked dependencies differ"));
    let extra = FIXTURE_LOCK.replace(" \"libc\",\n", " \"libc\",\n \"libc\",\n");
    assert!(validate(&extra, FIXTURE_MANIFEST).unwrap_err().contains("root locked dependencies differ"));
}

#[test]
fn rejects_malformed_lock_tables_and_non_v4_formats() {
    for bad in ["3", "5", "\"4\""] {
        let altered = FIXTURE_LOCK.replacen("version = 4", &format!("version = {bad}"), 1);
        assert!(validate(&altered, FIXTURE_MANIFEST).is_err());
    }
    assert!(validate(&FIXTURE_LOCK.replace("[[package]]", "[[package.other]]"), FIXTURE_MANIFEST).is_err());
    assert!(validate(&FIXTURE_LOCK.replace("dependencies = [", "dependencies = [ \"libc\" ]"), FIXTURE_MANIFEST).is_err());
}

#[test]
fn rejects_unreviewed_manifest_dependency_sections() {
    let injected = format!("{FIXTURE_MANIFEST}\n[target.'cfg(unix)'.dependencies]\nmalicious = \"*\"\n");
    assert!(validate(FIXTURE_LOCK, &injected).unwrap_err().contains("unapproved manifest section"));
}
