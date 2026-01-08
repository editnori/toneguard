use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::arch::Language;

fn parse_toml_string_value(raw: &str) -> Option<String> {
    let before_comment = raw.split('#').next().unwrap_or(raw).trim();
    let eq = before_comment.find('=')?;
    let mut value = before_comment[eq + 1..].trim();

    if value.starts_with("r#\"") {
        value = value.strip_prefix("r#\"")?;
        let end = value.find("\"#")?;
        return Some(value[..end].to_string());
    }

    let quote = value.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let tail = &value[quote.len_utf8()..];
    let end = tail.find(quote)?;
    Some(tail[..end].to_string())
}

fn toml_section_lines<'a>(text: &'a str, section: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut in_section = false;
    let header = format!("[{section}]");
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_section = trimmed == header;
            continue;
        }
        if in_section {
            out.push(line);
        }
    }
    out
}

fn parse_manifest_name(text: &str) -> Option<String> {
    for line in toml_section_lines(text, "package") {
        let trimmed = line.trim();
        if trimmed.starts_with("name") {
            return parse_toml_string_value(trimmed);
        }
    }
    None
}

fn parse_manifest_lib_name(text: &str) -> Option<String> {
    for line in toml_section_lines(text, "lib") {
        let trimmed = line.trim();
        if trimmed.starts_with("name") {
            return parse_toml_string_value(trimmed);
        }
    }
    None
}

fn parse_manifest_crate_name(text: &str) -> Option<String> {
    if let Some(name) = parse_manifest_lib_name(text) {
        return Some(name);
    }
    parse_manifest_name(text).map(|name| name.replace('-', "_"))
}

fn parse_workspace_members(text: &str) -> Vec<String> {
    let mut buf = String::new();
    let mut started = false;

    for line in toml_section_lines(text, "workspace") {
        let trimmed = line.trim();
        if trimmed.starts_with("members") {
            started = true;
            buf.push_str(trimmed);
            buf.push('\n');
            if trimmed.contains(']') {
                break;
            }
            continue;
        }
        if started {
            buf.push_str(trimmed);
            buf.push('\n');
            if trimmed.contains(']') {
                break;
            }
        }
    }

    if buf.is_empty() {
        return Vec::new();
    }

    let start = match buf.find('[') {
        Some(idx) => idx,
        None => return Vec::new(),
    };
    let end = match buf[start..].find(']') {
        Some(idx) => start + idx,
        None => return Vec::new(),
    };
    let inner = &buf[start + 1..end];

    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote: Option<char> = None;

    for ch in inner.chars() {
        if let Some(q) = in_quote {
            if ch == q {
                out.push(cur.clone());
                cur.clear();
                in_quote = None;
            } else {
                cur.push(ch);
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            in_quote = Some(ch);
        }
    }

    out.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn discover_workspace_crates(workspace_root: &Path) -> BTreeMap<String, PathBuf> {
    let manifest = workspace_root.join("Cargo.toml");
    let Ok(text) = std::fs::read_to_string(&manifest) else {
        return BTreeMap::new();
    };

    let members = parse_workspace_members(&text);
    let mut map = BTreeMap::new();

    if members.is_empty() {
        let Some(crate_name) = parse_manifest_crate_name(&text) else {
            return map;
        };
        let start = if workspace_root.join("src").is_dir() {
            workspace_root.join("src")
        } else {
            workspace_root.to_path_buf()
        };
        map.insert(crate_name, start);
        return map;
    }

    for member in members {
        let crate_root = workspace_root.join(member);
        let manifest_path = crate_root.join("Cargo.toml");
        let Ok(member_text) = std::fs::read_to_string(&manifest_path) else {
            continue;
        };
        let Some(crate_name) = parse_manifest_crate_name(&member_text) else {
            continue;
        };
        let start = if crate_root.join("src").is_dir() {
            crate_root.join("src")
        } else {
            crate_root.clone()
        };
        map.insert(crate_name, start);
    }

    map
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintConfig {
    pub ignore_globs: Vec<String>,
    pub base_dir: Option<PathBuf>,
}

impl Default for BlueprintConfig {
    fn default() -> Self {
        Self {
            ignore_globs: Vec::new(),
            base_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintNode {
    pub path: String,
    pub abs_path: String,
    pub language: Language,
    pub size_bytes: u64,
    pub lines: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeKind {
    Mod,
    Use,
    Import,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintEdge {
    pub from: String,
    pub to: Option<String>,
    pub to_raw: String,
    pub kind: EdgeKind,
    pub line: Option<u32>,
    pub resolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BlueprintStats {
    pub files_scanned: usize,
    pub nodes: usize,
    pub edges: usize,
    pub edges_resolved: usize,
    pub by_language: BTreeMap<String, usize>,
    pub by_edge_kind: BTreeMap<String, usize>,
    pub by_edge_kind_resolved: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintError {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintReport {
    pub nodes: Vec<BlueprintNode>,
    pub edges: Vec<BlueprintEdge>,
    pub stats: BlueprintStats,
    pub errors: Vec<BlueprintError>,
}

fn build_ignore_set(patterns: &[String]) -> Result<Option<GlobSet>> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern)?);
    }
    Ok(Some(builder.build()?))
}

fn language_for_path(path: &Path) -> Option<Language> {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase())
    {
        Some(ext) if ext == "rs" => Some(Language::Rust),
        Some(ext) if ext == "ts" || ext == "tsx" => Some(Language::TypeScript),
        Some(ext) if ext == "js" || ext == "jsx" => Some(Language::JavaScript),
        Some(ext) if ext == "py" => Some(Language::Python),
        _ => None,
    }
}

fn to_display_path(path: &Path, base_dir: &Option<PathBuf>) -> String {
    // Keep display paths stable regardless of whether the scanner walked an absolute or relative root.
    // If we have a base_dir, normalize relative paths under it before stripping the prefix.
    let mut candidate = path.to_path_buf();
    if candidate.is_relative() {
        if let Some(base) = base_dir {
            candidate = base.join(candidate);
        }
    }

    let raw = if let Some(base) = base_dir {
        candidate
            .strip_prefix(base)
            .unwrap_or(candidate.as_path())
            .to_string_lossy()
            .to_string()
    } else {
        candidate.to_string_lossy().to_string()
    };

    let normalized = raw.replace('\\', "/");
    normalized
        .strip_prefix("./")
        .unwrap_or(normalized.as_str())
        .to_string()
}

fn collect_code_files(paths: &[PathBuf], ignore: Option<&GlobSet>) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let roots = if paths.is_empty() {
        vec![std::env::current_dir()?]
    } else {
        paths.to_vec()
    };
    for root in roots {
        if root.is_dir() {
            let mut walker = WalkDir::new(&root).into_iter();
            while let Some(entry_res) = walker.next() {
                let entry = entry_res?;
                let path = entry.path();
                if let Some(set) = ignore {
                    if set.is_match(path) {
                        if entry.file_type().is_dir() {
                            walker.skip_current_dir();
                        }
                        continue;
                    }
                }
                if entry.file_type().is_dir() {
                    continue;
                }
                if entry.file_type().is_file() && language_for_path(path).is_some() {
                    files.push(path.to_path_buf());
                }
            }
        } else if root.is_file() && language_for_path(&root).is_some() {
            if let Some(set) = ignore {
                if set.is_match(&root) {
                    continue;
                }
            }
            files.push(root);
        }
    }
    Ok(files)
}

#[derive(Debug, Clone)]
struct RawEdge {
    to_path: Option<PathBuf>,
    to_raw: String,
    kind: EdgeKind,
    line: Option<u32>,
}

pub fn blueprint_paths(paths: &[PathBuf], config: &BlueprintConfig) -> Result<BlueprintReport> {
    let ignore_set = build_ignore_set(&config.ignore_globs)?;
    let files = collect_code_files(paths, ignore_set.as_ref())?;

    let base_dir = config
        .base_dir
        .clone()
        .or_else(|| std::env::current_dir().ok());
    let workspace_root = base_dir.clone().unwrap_or_else(|| PathBuf::from("."));
    let workspace_crates = discover_workspace_crates(&workspace_root);

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut errors = Vec::new();

    let mut scanned_abs: BTreeSet<PathBuf> = BTreeSet::new();
    for path in &files {
        if let Ok(canon) = path.canonicalize() {
            scanned_abs.insert(canon);
        } else {
            scanned_abs.insert(path.to_path_buf());
        }
    }

    for path in &files {
        let language = match language_for_path(path) {
            Some(lang) => lang,
            None => continue,
        };

        let (size_bytes, content) = match std::fs::metadata(path)
            .and_then(|meta| Ok((meta.len(), std::fs::read_to_string(path)?)))
        {
            Ok((size, text)) => (size, text),
            Err(err) => {
                errors.push(BlueprintError {
                    path: to_display_path(path, &base_dir),
                    message: format!("{err}"),
                });
                continue;
            }
        };

        let line_count = content.lines().count().max(1) as u32;
        let abs_path = path
            .canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .to_string();
        let display = to_display_path(path, &base_dir);

        nodes.push(BlueprintNode {
            path: display.clone(),
            abs_path,
            language: language.clone(),
            size_bytes,
            lines: line_count,
        });

        let raw_edges = extract_edges(language, path, &content, &workspace_root, &workspace_crates);
        for raw in raw_edges {
            let mut resolved = false;
            let to_display = raw.to_path.as_ref().and_then(|p| {
                let abs = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
                if scanned_abs.contains(&abs) {
                    resolved = true;
                    Some(to_display_path(&abs, &base_dir))
                } else {
                    None
                }
            });

            edges.push(BlueprintEdge {
                from: display.clone(),
                to: to_display,
                to_raw: raw.to_raw,
                kind: raw.kind,
                line: raw.line,
                resolved,
            });
        }
    }

    nodes.sort_by(|a, b| a.path.cmp(&b.path));
    edges.sort_by(|a, b| {
        a.from
            .cmp(&b.from)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.to_raw.cmp(&b.to_raw))
            .then_with(|| a.line.cmp(&b.line))
    });

    let mut stats = BlueprintStats::default();
    stats.files_scanned = files.len();
    stats.nodes = nodes.len();
    stats.edges = edges.len();
    stats.edges_resolved = edges.iter().filter(|e| e.resolved).count();

    for node in &nodes {
        *stats
            .by_language
            .entry(format!("{:?}", node.language).to_lowercase())
            .or_default() += 1;
    }
    for edge in &edges {
        *stats
            .by_edge_kind
            .entry(format!("{:?}", edge.kind).to_lowercase())
            .or_default() += 1;
        if edge.resolved {
            *stats
                .by_edge_kind_resolved
                .entry(format!("{:?}", edge.kind).to_lowercase())
                .or_default() += 1;
        }
    }

    Ok(BlueprintReport {
        nodes,
        edges,
        stats,
        errors,
    })
}

fn extract_edges(
    language: Language,
    from_path: &Path,
    content: &str,
    workspace_root: &Path,
    workspace_crates: &BTreeMap<String, PathBuf>,
) -> Vec<RawEdge> {
    match language {
        Language::Rust => extract_rust_edges(from_path, content, workspace_root, workspace_crates),
        Language::TypeScript | Language::JavaScript => extract_js_edges(from_path, content),
        Language::Python => extract_python_edges(from_path, content),
    }
}

fn extract_js_edges(from_path: &Path, content: &str) -> Vec<RawEdge> {
    static RE_IMPORT_FROM: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"^\s*import\s+(?:type\s+)?[^;]*?\s+from\s+['"]([^'"]+)['"]"#).unwrap()
    });
    static RE_IMPORT_SIDE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r#"^\s*import\s+['"]([^'"]+)['"]"#).unwrap());
    static RE_EXPORT_FROM: Lazy<Regex> =
        Lazy::new(|| Regex::new(r#"^\s*export\s+[^;]*?\s+from\s+['"]([^'"]+)['"]"#).unwrap());
    static RE_REQUIRE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r#"require\(\s*['"]([^'"]+)['"]\s*\)"#).unwrap());
    static RE_DYNAMIC_IMPORT: Lazy<Regex> =
        Lazy::new(|| Regex::new(r#"import\(\s*['"]([^'"]+)['"]\s*\)"#).unwrap());

    let mut edges = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let mut modules = Vec::new();
        if let Some(cap) = RE_IMPORT_FROM.captures(line) {
            modules.push(cap[1].to_string());
        } else if let Some(cap) = RE_IMPORT_SIDE.captures(line) {
            modules.push(cap[1].to_string());
        } else if let Some(cap) = RE_EXPORT_FROM.captures(line) {
            modules.push(cap[1].to_string());
        }
        for cap in RE_REQUIRE.captures_iter(line) {
            modules.push(cap[1].to_string());
        }
        for cap in RE_DYNAMIC_IMPORT.captures_iter(line) {
            modules.push(cap[1].to_string());
        }

        for module in modules {
            if !module.starts_with('.') {
                continue;
            }
            let resolved = resolve_js_module(from_path, &module);
            edges.push(RawEdge {
                to_path: resolved,
                to_raw: module,
                kind: EdgeKind::Import,
                line: Some(line_no),
            });
        }
    }
    edges
}

fn resolve_js_module(from_path: &Path, module: &str) -> Option<PathBuf> {
    let base_dir = from_path.parent().unwrap_or_else(|| Path::new("."));
    let joined = base_dir.join(module);
    resolve_js_path(&joined)
}

fn resolve_js_path(base: &Path) -> Option<PathBuf> {
    let exts = ["ts", "tsx", "js", "jsx", "mjs", "cjs"];
    let mut candidates = Vec::new();

    if base.extension().is_some() {
        candidates.push(base.to_path_buf());
    } else {
        for ext in exts {
            candidates.push(base.with_extension(ext));
        }
    }

    for ext in exts {
        candidates.push(base.join(format!("index.{ext}")));
    }

    for cand in candidates {
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

fn extract_python_edges(from_path: &Path, content: &str) -> Vec<RawEdge> {
    static RE_FROM_IMPORT: Lazy<Regex> =
        Lazy::new(|| Regex::new(r#"^\s*from\s+([.]+)([A-Za-z0-9_\.]*)\s+import\s+"#).unwrap());

    let mut edges = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let Some(cap) = RE_FROM_IMPORT.captures(line) else {
            continue;
        };
        let dots = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let rest = cap.get(2).map(|m| m.as_str()).unwrap_or("");
        let resolved = resolve_python_relative(from_path, dots, rest);
        let raw = format!("{dots}{rest}");
        edges.push(RawEdge {
            to_path: resolved,
            to_raw: raw,
            kind: EdgeKind::Import,
            line: Some(line_no),
        });
    }
    edges
}

fn resolve_python_relative(from_path: &Path, dots: &str, module: &str) -> Option<PathBuf> {
    if dots.is_empty() {
        return None;
    }
    let mut base = from_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let up = dots.len().saturating_sub(1);
    for _ in 0..up {
        base = base.parent()?.to_path_buf();
    }

    if module.is_empty() {
        let init = base.join("__init__.py");
        if init.is_file() {
            return Some(init);
        }
        return None;
    }

    let rel = module.replace('.', "/");
    let base_mod = base.join(rel);
    let direct = base_mod.with_extension("py");
    if direct.is_file() {
        return Some(direct);
    }
    let init = base_mod.join("__init__.py");
    if init.is_file() {
        return Some(init);
    }
    None
}

fn extract_rust_edges(
    from_path: &Path,
    content: &str,
    workspace_root: &Path,
    workspace_crates: &BTreeMap<String, PathBuf>,
) -> Vec<RawEdge> {
    static RE_MOD: Lazy<Regex> =
        Lazy::new(|| Regex::new(r#"^\s*(?:pub\s+)?mod\s+([A-Za-z0-9_]+)\s*;\s*$"#).unwrap());

    let mut edges = Vec::new();

    for (idx, line) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let Some(cap) = RE_MOD.captures(line) else {
            continue;
        };
        let name = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let resolved = resolve_rust_mod_decl(from_path, name);
        edges.push(RawEdge {
            to_path: resolved,
            to_raw: format!("mod {name}"),
            kind: EdgeKind::Mod,
            line: Some(line_no),
        });
    }

    edges.extend(extract_rust_use_edges(
        from_path,
        content,
        workspace_root,
        workspace_crates,
    ));

    edges
}

fn resolve_rust_mod_decl(from_path: &Path, name: &str) -> Option<PathBuf> {
    let base = from_path.parent().unwrap_or_else(|| Path::new("."));
    let direct = base.join(format!("{name}.rs"));
    if direct.is_file() {
        return Some(direct);
    }
    let mod_rs = base.join(name).join("mod.rs");
    if mod_rs.is_file() {
        return Some(mod_rs);
    }
    None
}

fn extract_rust_use_edges(
    from_path: &Path,
    content: &str,
    workspace_root: &Path,
    workspace_crates: &BTreeMap<String, PathBuf>,
) -> Vec<RawEdge> {
    let mut edges = Vec::new();
    let mut stmt = String::new();
    let mut stmt_line: Option<u32> = None;

    for (idx, line) in content.lines().enumerate() {
        let raw = line;
        let trimmed = raw.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }

        if stmt_line.is_some() {
            stmt.push_str(trimmed);
            stmt.push('\n');
            if trimmed.contains(';') {
                edges.extend(process_rust_use_stmt(
                    from_path,
                    &stmt,
                    stmt_line,
                    workspace_root,
                    workspace_crates,
                ));
                stmt.clear();
                stmt_line = None;
            }
            continue;
        }

        if trimmed.starts_with("use ") || trimmed.starts_with("pub use ") {
            stmt_line = Some((idx + 1) as u32);
            stmt.push_str(trimmed);
            stmt.push('\n');
            if trimmed.contains(';') {
                edges.extend(process_rust_use_stmt(
                    from_path,
                    &stmt,
                    stmt_line,
                    workspace_root,
                    workspace_crates,
                ));
                stmt.clear();
                stmt_line = None;
            }
        }
    }

    edges
}

fn process_rust_use_stmt(
    from_path: &Path,
    stmt: &str,
    line: Option<u32>,
    workspace_root: &Path,
    workspace_crates: &BTreeMap<String, PathBuf>,
) -> Vec<RawEdge> {
    let mut expr = stmt.trim().to_string();
    if expr.starts_with("pub use ") {
        expr = expr.trim_start_matches("pub ").to_string();
    }
    if !expr.starts_with("use ") {
        return Vec::new();
    }
    expr = expr.trim_start_matches("use ").to_string();
    if let Some(idx) = expr.find(';') {
        expr.truncate(idx);
    }
    let expr = expr.trim();
    if expr.is_empty() {
        return Vec::new();
    }

    let targets = expand_rust_use_targets(expr);
    let mut edges = Vec::new();
    for target in targets {
        let resolved =
            resolve_rust_use_target(from_path, &target, workspace_root, workspace_crates);
        edges.push(RawEdge {
            to_path: resolved,
            to_raw: target,
            kind: EdgeKind::Use,
            line,
        });
    }
    edges
}

fn expand_rust_use_targets(expr: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = expr.as_bytes();
    let mut brace_start = None;
    let mut brace_end = None;
    let mut depth = 0usize;
    for (idx, b) in bytes.iter().enumerate() {
        match b {
            b'{' => {
                if depth == 0 {
                    brace_start = Some(idx);
                }
                depth += 1;
            }
            b'}' => {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        brace_end = Some(idx);
                        break;
                    }
                }
            }
            _ => {}
        }
    }

    let Some(start) = brace_start else {
        return vec![expr.trim().to_string()];
    };
    let Some(end) = brace_end else {
        return vec![expr.trim().to_string()];
    };

    let prefix = expr[..start].trim().trim_end_matches("::").trim();
    let inner = expr[start + 1..end].trim();
    if inner.is_empty() {
        return vec![prefix.to_string()];
    }

    let mut cur = String::new();
    let mut inner_depth = 0usize;
    let mut items = Vec::new();
    for ch in inner.chars() {
        match ch {
            '{' => {
                inner_depth += 1;
                cur.push(ch);
            }
            '}' => {
                inner_depth = inner_depth.saturating_sub(1);
                cur.push(ch);
            }
            ',' if inner_depth == 0 => {
                let trimmed = cur.trim();
                if !trimmed.is_empty() {
                    items.push(trimmed.to_string());
                }
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    let trimmed = cur.trim();
    if !trimmed.is_empty() {
        items.push(trimmed.to_string());
    }

    for item in items {
        let item = item.trim();
        if item.is_empty() || item == "*" {
            continue;
        }
        let item = item
            .split(" as ")
            .next()
            .unwrap_or(item)
            .trim()
            .trim_start_matches("::");
        if item.is_empty() {
            continue;
        }
        if item == "self" {
            out.push(prefix.to_string());
            continue;
        }
        if prefix.is_empty() {
            out.push(item.to_string());
        } else {
            out.push(format!("{prefix}::{item}"));
        }
    }

    if out.is_empty() {
        out.push(prefix.to_string());
    }

    out
}

fn resolve_rust_root_file(start: &Path) -> Option<PathBuf> {
    let lib = start.join("lib.rs");
    if lib.is_file() {
        return Some(lib);
    }
    let main = start.join("main.rs");
    if main.is_file() {
        return Some(main);
    }
    None
}

fn resolve_rust_use_target(
    from_path: &Path,
    target: &str,
    workspace_root: &Path,
    workspace_crates: &BTreeMap<String, PathBuf>,
) -> Option<PathBuf> {
    let target = target.trim().trim_start_matches("::");
    if target.is_empty() {
        return None;
    }

    let (start, mut segments) =
        parse_rust_module_start(from_path, target, workspace_root, workspace_crates)?;
    if segments.is_empty() {
        return resolve_rust_root_file(&start);
    }

    // Try longest prefix first, then back off.
    let start_dir = start.clone();
    while !segments.is_empty() {
        let rel = segments.join("/");
        let base = start.join(rel);
        let direct = base.with_extension("rs");
        if direct.is_file() {
            return Some(direct);
        }
        let mod_rs = base.join("mod.rs");
        if mod_rs.is_file() {
            return Some(mod_rs);
        }
        segments.pop();
    }
    resolve_rust_root_file(&start_dir)
}

fn parse_rust_module_start(
    from_path: &Path,
    target: &str,
    workspace_root: &Path,
    workspace_crates: &BTreeMap<String, PathBuf>,
) -> Option<(PathBuf, Vec<String>)> {
    let from_dir = from_path.parent().unwrap_or_else(|| Path::new("."));
    if let Some(rest) = target.strip_prefix("crate::") {
        let crate_root = find_rust_crate_root(from_path, workspace_root)
            .unwrap_or_else(|| workspace_root.to_path_buf());
        let src_dir = crate_root.join("src");
        let start = if src_dir.is_dir() {
            src_dir
        } else {
            crate_root
        };
        let segments = split_rust_segments(rest);
        return Some((start, segments));
    }
    if let Some(rest) = target.strip_prefix("self::") {
        let segments = split_rust_segments(rest);
        return Some((from_dir.to_path_buf(), segments));
    }
    if target.starts_with("super::") {
        let mut rest = target;
        let mut up = 0usize;
        while let Some(next) = rest.strip_prefix("super::") {
            up += 1;
            rest = next;
        }
        let mut start = from_dir.to_path_buf();
        for _ in 0..up {
            start = start.parent()?.to_path_buf();
        }
        let segments = split_rust_segments(rest);
        return Some((start, segments));
    }

    if let Some((prefix, rest)) = target.split_once("::") {
        if let Some(start) = workspace_crates.get(prefix) {
            let segments = split_rust_segments(rest);
            return Some((start.clone(), segments));
        }
    } else if let Some(start) = workspace_crates.get(target) {
        return Some((start.clone(), Vec::new()));
    }
    None
}

fn split_rust_segments(rest: &str) -> Vec<String> {
    rest.split("::")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn find_rust_crate_root(from_path: &Path, workspace_root: &Path) -> Option<PathBuf> {
    let mut dir = from_path.parent();
    while let Some(cur) = dir {
        if cur.join("Cargo.toml").is_file() {
            return Some(cur.to_path_buf());
        }
        if cur == workspace_root {
            break;
        }
        dir = cur.parent();
    }
    None
}
