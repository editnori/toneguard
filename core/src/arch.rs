use std::collections::{hash_map::DefaultHasher, BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FindingCategory {
    Placeholder,
    LonelyAbstraction,
    PassThrough,
    Duplication,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FindingSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FindingConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowFinding {
    pub category: FindingCategory,
    pub severity: FindingSeverity,
    pub confidence: FindingConfidence,
    pub message: String,
    pub path: String,
    pub line: Option<u32>,
    pub symbol: Option<String>,
    pub language: Language,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlowAuditSummary {
    pub files_scanned: usize,
    pub findings: usize,
    pub by_category: BTreeMap<String, usize>,
    pub by_language: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowAuditReport {
    pub summary: FlowAuditSummary,
    pub findings: Vec<FlowFinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FlowAuditConfig {
    pub ignore_globs: Vec<String>,
    pub languages: Vec<Language>,
    pub max_file_kb: Option<u64>,
    pub base_dir: Option<PathBuf>,
    pub duplication_min_instances: usize,
    pub duplication_min_tokens: usize,
    pub duplication_max_groups: usize,
}

impl Default for FlowAuditConfig {
    fn default() -> Self {
        Self {
            ignore_globs: Vec::new(),
            languages: vec![
                Language::Rust,
                Language::TypeScript,
                Language::JavaScript,
                Language::Python,
            ],
            max_file_kb: Some(512),
            base_dir: None,
            duplication_min_instances: 3,
            duplication_min_tokens: 80,
            duplication_max_groups: 20,
        }
    }
}

#[derive(Clone)]
struct DupSignature {
    language: Language,
    normalized: String,
    token_count: usize,
    path: PathBuf,
    line: Option<u32>,
    symbol: Option<String>,
}

#[derive(Clone)]
struct DupOccurrence {
    path: PathBuf,
    line: Option<u32>,
    symbol: Option<String>,
}

struct DupGroup {
    language: Language,
    normalized: String,
    token_count: usize,
    occurrences: Vec<DupOccurrence>,
}

pub fn audit_paths(paths: &[PathBuf], config: &FlowAuditConfig) -> Result<FlowAuditReport> {
    let ignore_set = build_ignore_set(&config.ignore_globs)?;
    let files = collect_code_files(paths, &ignore_set, config)?;

    let mut findings = Vec::new();
    let mut dup_signatures = Vec::new();
    let mut rust_aggregate = RustAggregate::default();
    let mut ts_aggregate = TsAggregate::default();
    let mut py_aggregate = PyAggregate::default();

    for file in &files {
        let language = match language_for_path(file) {
            Some(lang) => lang,
            None => continue,
        };
        if !config.languages.contains(&language) {
            continue;
        }
        let text = std::fs::read_to_string(file)
            .with_context(|| format!("Failed to read {}", file.display()))?;
        match language {
            Language::Rust => {
                let report = analyze_rust_file(file, &text);
                dup_signatures.extend(report.dup_signatures.clone());
                rust_aggregate.absorb(report);
            }
            Language::TypeScript | Language::JavaScript => {
                let report = analyze_ts_file(file, &text, &language);
                dup_signatures.extend(report.dup_signatures.clone());
                ts_aggregate.absorb(report);
            }
            Language::Python => {
                let report = analyze_py_file(file, &text);
                dup_signatures.extend(report.dup_signatures.clone());
                py_aggregate.absorb(report);
            }
        }
    }

    rust_aggregate.emit_findings(&mut findings, config);
    ts_aggregate.emit_findings(&mut findings, config);
    py_aggregate.emit_findings(&mut findings, config);
    emit_duplication_findings(&dup_signatures, &mut findings, config);

    let summary = summarize(&findings, files.len());
    Ok(FlowAuditReport { summary, findings })
}

fn summarize(findings: &[FlowFinding], files_scanned: usize) -> FlowAuditSummary {
    let mut summary = FlowAuditSummary::default();
    summary.files_scanned = files_scanned;
    summary.findings = findings.len();
    for finding in findings {
        let cat = format!("{:?}", finding.category).to_lowercase();
        *summary.by_category.entry(cat).or_insert(0) += 1;
        let lang = format!("{:?}", finding.language).to_lowercase();
        *summary.by_language.entry(lang).or_insert(0) += 1;
    }
    summary
}

fn emit_duplication_findings(
    signatures: &[DupSignature],
    findings: &mut Vec<FlowFinding>,
    config: &FlowAuditConfig,
) {
    if signatures.is_empty() || config.duplication_min_instances == 0 {
        return;
    }

    let mut buckets: HashMap<(Language, u64), Vec<DupGroup>> = HashMap::new();
    for signature in signatures {
        if signature.token_count < config.duplication_min_tokens {
            continue;
        }
        let hash = stable_hash64(&signature.normalized);
        let key = (signature.language.clone(), hash);
        let bucket = buckets.entry(key).or_default();
        if let Some(group) = bucket
            .iter_mut()
            .find(|g| g.normalized == signature.normalized)
        {
            group.occurrences.push(DupOccurrence {
                path: signature.path.clone(),
                line: signature.line,
                symbol: signature.symbol.clone(),
            });
            continue;
        }

        bucket.push(DupGroup {
            language: signature.language.clone(),
            normalized: signature.normalized.clone(),
            token_count: signature.token_count,
            occurrences: vec![DupOccurrence {
                path: signature.path.clone(),
                line: signature.line,
                symbol: signature.symbol.clone(),
            }],
        });
    }

    let mut groups: Vec<DupGroup> = buckets
        .into_values()
        .flatten()
        .filter(|g| g.occurrences.len() >= config.duplication_min_instances)
        .collect();
    groups.sort_by(|a, b| b.occurrences.len().cmp(&a.occurrences.len()));

    for group in groups.into_iter().take(config.duplication_max_groups) {
        let first = group.occurrences.first();
        let (path, line, symbol) = if let Some(first) = first {
            (
                to_display_path(&first.path, config),
                first.line,
                first.symbol.clone(),
            )
        } else {
            ("<unknown>".into(), None, None)
        };

        let mut evidence = Vec::new();
        for occ in group.occurrences.iter().take(12) {
            let occ_path = to_display_path(&occ.path, config);
            let occ_line = occ
                .line
                .map(|l| format!(":{}", l))
                .unwrap_or_else(|| "".into());
            let occ_symbol = occ
                .symbol
                .as_ref()
                .map(|s| format!(" ({})", s))
                .unwrap_or_default();
            evidence.push(format!("{occ_path}{occ_line}{occ_symbol}"));
        }
        if group.occurrences.len() > 12 {
            evidence.push(format!("... and {} more", group.occurrences.len() - 12));
        }

        findings.push(FlowFinding {
            category: FindingCategory::Duplication,
            severity: FindingSeverity::Info,
            confidence: FindingConfidence::Medium,
            message: format!(
                "Duplicated logic (~{} tokens) appears {} times; consider extracting an abstraction (reason: reuse).",
                group.token_count,
                group.occurrences.len()
            ),
            path,
            line,
            symbol,
            language: group.language,
            evidence,
        });
    }
}

fn stable_hash64(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn normalize_code_for_duplication(fragment: &str) -> (String, usize) {
    fn push(out: &mut String, token: &str, count: &mut usize) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(token);
        *count += 1;
    }

    fn is_ident_start(byte: u8) -> bool {
        byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$'
    }

    fn is_ident_continue(byte: u8) -> bool {
        is_ident_start(byte) || byte.is_ascii_digit()
    }

    fn is_keyword(token: &str) -> bool {
        matches!(
            token,
            // Rust
            "as"
                | "async"
                | "await"
                | "break"
                | "const"
                | "continue"
                | "crate"
                | "dyn"
                | "else"
                | "enum"
                | "extern"
                | "false"
                | "fn"
                | "for"
                | "if"
                | "impl"
                | "in"
                | "let"
                | "loop"
                | "match"
                | "mod"
                | "move"
                | "mut"
                | "pub"
                | "ref"
                | "return"
                | "self"
                | "Self"
                | "static"
                | "struct"
                | "super"
                | "trait"
                | "true"
                | "type"
                | "unsafe"
                | "use"
                | "where"
                // JS/TS
                | "case"
                | "catch"
                | "class"
                | "default"
                | "export"
                | "extends"
                | "finally"
                | "from"
                | "function"
                | "get"
                | "import"
                | "interface"
                | "implements"
                | "new"
                | "null"
                | "private"
                | "protected"
                | "public"
                | "set"
                | "switch"
                | "this"
                | "throw"
                | "try"
                | "undefined"
                | "var"
                // Python
                | "and"
                | "def"
                | "elif"
                | "except"
                | "False"
                | "is"
                | "lambda"
                | "None"
                | "not"
                | "or"
                | "pass"
                | "raise"
                | "True"
                | "with"
                | "yield"
        )
    }

    let bytes = fragment.as_bytes();
    let mut out = String::new();
    let mut token_count = 0usize;
    let mut i = 0usize;
    let mut at_line_start = true;

    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            if b == b'\n' || b == b'\r' {
                at_line_start = true;
            }
            i += 1;
            continue;
        }

        if b == b'/' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if next == b'/' {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if next == b'*' {
                i += 2;
                while i + 1 < bytes.len() {
                    if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        i += 2;
                        break;
                    }
                    if bytes[i] == b'\n' {
                        at_line_start = true;
                    }
                    i += 1;
                }
                continue;
            }
        }

        if b == b'#' && at_line_start {
            i += 1;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        at_line_start = false;

        if b == b'\'' || b == b'"' || b == b'`' {
            let quote = b;
            if (quote == b'\'' || quote == b'"')
                && i + 2 < bytes.len()
                && bytes[i + 1] == quote
                && bytes[i + 2] == quote
            {
                i += 3;
                while i + 2 < bytes.len() {
                    if bytes[i] == quote && bytes[i + 1] == quote && bytes[i + 2] == quote {
                        i += 3;
                        break;
                    }
                    if bytes[i] == b'\\' {
                        i = (i + 2).min(bytes.len());
                        continue;
                    }
                    i += 1;
                }
            } else {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i = (i + 2).min(bytes.len());
                        continue;
                    }
                    if bytes[i] == quote {
                        i += 1;
                        break;
                    }
                    if bytes[i] == b'\n' {
                        at_line_start = true;
                    }
                    i += 1;
                }
            }
            push(&mut out, "\"\"", &mut token_count);
            continue;
        }

        if b.is_ascii_digit() {
            i += 1;
            while i < bytes.len() {
                let bb = bytes[i];
                if bb.is_ascii_alphanumeric() || bb == b'_' || bb == b'.' {
                    i += 1;
                } else {
                    break;
                }
            }
            push(&mut out, "0", &mut token_count);
            continue;
        }

        if is_ident_start(b) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                i += 1;
            }
            let token = fragment.get(start..i).unwrap_or_default();
            if is_keyword(token) {
                push(&mut out, token, &mut token_count);
            } else {
                push(&mut out, "id", &mut token_count);
            }
            continue;
        }

        if i + 1 < bytes.len() && bytes[i].is_ascii() && bytes[i + 1].is_ascii() {
            let pair = match (b, bytes[i + 1]) {
                (b'=', b'=') => Some("=="),
                (b'!', b'=') => Some("!="),
                (b'<', b'=') => Some("<="),
                (b'>', b'=') => Some(">="),
                (b'=', b'>') => Some("=>"),
                (b'-', b'>') => Some("->"),
                (b':', b':') => Some("::"),
                (b'&', b'&') => Some("&&"),
                (b'|', b'|') => Some("||"),
                (b'+', b'=') => Some("+="),
                (b'-', b'=') => Some("-="),
                (b'*', b'=') => Some("*="),
                (b'/', b'=') => Some("/="),
                (b'%', b'=') => Some("%="),
                (b'<', b'<') => Some("<<"),
                (b'>', b'>') => Some(">>"),
                (b':', b'=') => Some(":="),
                _ => None,
            };
            if let Some(token) = pair {
                push(&mut out, token, &mut token_count);
                i += 2;
                continue;
            }
        }

        let ch = fragment[i..].chars().next().unwrap_or('\0');
        if ch == '\0' {
            break;
        }
        push(&mut out, &ch.to_string(), &mut token_count);
        i += ch.len_utf8();
    }

    (out, token_count)
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

fn collect_code_files(
    paths: &[PathBuf],
    ignore: &Option<GlobSet>,
    config: &FlowAuditConfig,
) -> Result<Vec<PathBuf>> {
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
                    if !within_size_limit(path, config.max_file_kb) {
                        continue;
                    }
                    files.push(path.to_path_buf());
                }
            }
        } else if root.is_file() && language_for_path(&root).is_some() {
            if let Some(set) = ignore {
                if set.is_match(&root) {
                    continue;
                }
            }
            if within_size_limit(&root, config.max_file_kb) {
                files.push(root);
            }
        }
    }
    Ok(files)
}

fn within_size_limit(path: &Path, max_kb: Option<u64>) -> bool {
    let Some(limit_kb) = max_kb else {
        return true;
    };
    if let Ok(meta) = std::fs::metadata(path) {
        meta.len() <= limit_kb * 1024
    } else {
        false
    }
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

fn to_display_path(path: &Path, config: &FlowAuditConfig) -> String {
    if let Some(base) = &config.base_dir {
        if let Ok(rel) = path.strip_prefix(base) {
            return rel.to_string_lossy().replace('\\', "/");
        }
    }
    path.to_string_lossy().replace('\\', "/")
}

#[derive(Default)]
struct RustAggregate {
    trait_defs: Vec<RustTraitInfo>,
    trait_impls: HashMap<String, usize>,
    pass_maps: HashMap<PathBuf, HashMap<String, RustPassThrough>>,
    placeholder_findings: Vec<FlowFinding>,
}

impl RustAggregate {
    fn absorb(&mut self, report: RustFileReport) {
        self.trait_defs.extend(report.trait_defs);
        for trait_name in report.trait_impls {
            *self.trait_impls.entry(trait_name).or_insert(0) += 1;
        }
        if !report.pass_map.is_empty() {
            self.pass_maps.insert(report.path.clone(), report.pass_map);
        }
        self.placeholder_findings
            .extend(report.placeholder_findings);
    }

    fn emit_findings(&self, findings: &mut Vec<FlowFinding>, config: &FlowAuditConfig) {
        for trait_info in &self.trait_defs {
            if trait_info.allow_lonely {
                continue;
            }
            let count = self.trait_impls.get(&trait_info.name).copied().unwrap_or(0);
            if count <= 1 {
                findings.push(FlowFinding {
                    category: FindingCategory::LonelyAbstraction,
                    severity: if count == 0 {
                        FindingSeverity::Warning
                    } else {
                        FindingSeverity::Info
                    },
                    confidence: FindingConfidence::High,
                    message: format!(
                        "Trait `{}` has {} implementation{}.",
                        trait_info.name,
                        count,
                        if count == 1 { "" } else { "s" }
                    ),
                    path: to_display_path(&trait_info.path, config),
                    line: trait_info.line,
                    symbol: Some(trait_info.name.clone()),
                    language: Language::Rust,
                    evidence: vec!["Trait has <=1 impl in repo scan.".into()],
                });
            }
        }

        for (path, pass_map) in &self.pass_maps {
            // Build reverse map: which functions are callees (i.e., called by other pass-throughs)
            let mut reverse: HashMap<String, usize> = HashMap::new();
            for (_, info) in pass_map {
                *reverse.entry(info.callee.clone()).or_insert(0) += 1;
            }
            let mut seen = HashSet::new();
            for (caller, info) in pass_map {
                // Only start chains from functions that are NOT callees of other pass-throughs
                // (i.e., find the "roots" of pass-through chains)
                if reverse.contains_key(caller) {
                    continue;
                }
                if seen.contains(caller) {
                    continue;
                }
                let chain = build_pass_chain(caller, pass_map, |info| info.callee.as_str());
                if chain.len() >= 2 {
                    seen.extend(chain.iter().cloned());
                    let chain_msg = chain.join(" -> ");
                    findings.push(FlowFinding {
                        category: FindingCategory::PassThrough,
                        severity: FindingSeverity::Info,
                        confidence: FindingConfidence::High,
                        message: format!(
                            "Pass-through wrapper chain length {}: {}",
                            chain.len(),
                            chain_msg
                        ),
                        path: to_display_path(path, config),
                        line: info.line,
                        symbol: Some(caller.clone()),
                        language: Language::Rust,
                        evidence: vec![format!("Forward-only functions: {}", chain_msg)],
                    });
                }
            }
        }

        for mut finding in self.placeholder_findings.clone() {
            normalize_finding_path(&mut finding, config);
            findings.push(finding);
        }
    }
}

#[derive(Clone)]
struct RustPassThrough {
    callee: String,
    line: Option<u32>,
}

#[derive(Clone)]
struct RustTraitInfo {
    name: String,
    path: PathBuf,
    line: Option<u32>,
    allow_lonely: bool,
}

struct RustFileReport {
    path: PathBuf,
    trait_defs: Vec<RustTraitInfo>,
    trait_impls: Vec<String>,
    pass_map: HashMap<String, RustPassThrough>,
    placeholder_findings: Vec<FlowFinding>,
    dup_signatures: Vec<DupSignature>,
}

fn build_pass_chain<T>(
    start: &str,
    pass_map: &HashMap<String, T>,
    callee_of: impl Fn(&T) -> &str,
) -> Vec<String> {
    let mut chain = Vec::new();
    let mut current = start.to_string();
    let mut guard = 0;
    while guard < 20 {
        guard += 1;
        if chain.contains(&current) {
            break;
        }
        chain.push(current.clone());
        let Some(next) = pass_map.get(&current) else {
            break;
        };
        current = callee_of(next).to_string();
    }
    chain
}

fn analyze_rust_file(path: &Path, text: &str) -> RustFileReport {
    use quote::ToTokens;
    use syn::visit::Visit;

    let mut trait_defs = Vec::new();
    let mut trait_impls = Vec::new();
    let mut pass_map = HashMap::new();
    let mut placeholder_findings = Vec::new();
    let mut dup_signatures = Vec::new();

    let parsed = syn::parse_file(text);
    let Ok(file) = parsed else {
        return RustFileReport {
            path: path.to_path_buf(),
            trait_defs,
            trait_impls,
            pass_map,
            placeholder_findings,
            dup_signatures,
        };
    };

    struct TraitVisitor {
        trait_defs: Vec<RustTraitInfo>,
        trait_impls: Vec<String>,
        file_path: PathBuf,
    }

    impl<'ast> Visit<'ast> for TraitVisitor {
        fn visit_item_trait(&mut self, i: &'ast syn::ItemTrait) {
            let name = i.ident.to_string();
            let allow_lonely = has_allow_marker(&i.attrs);
            let line = i.ident.span().start().line;
            self.trait_defs.push(RustTraitInfo {
                name,
                path: self.file_path.clone(),
                line: Some(line as u32),
                allow_lonely,
            });
            syn::visit::visit_item_trait(self, i);
        }

        fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
            if let Some((_, path, _)) = &i.trait_ {
                let trait_name = path
                    .segments
                    .last()
                    .map(|seg| seg.ident.to_string())
                    .unwrap_or_else(|| "UnknownTrait".into());
                self.trait_impls.push(trait_name);
            }
            syn::visit::visit_item_impl(self, i);
        }
    }

    let mut trait_visitor = TraitVisitor {
        trait_defs: Vec::new(),
        trait_impls: Vec::new(),
        file_path: path.to_path_buf(),
    };
    trait_visitor.visit_file(&file);
    trait_defs = trait_visitor.trait_defs;
    trait_impls = trait_visitor.trait_impls;

    for item in file.items {
        match item {
            syn::Item::Fn(item_fn) => {
                let fn_name = item_fn.sig.ident.to_string();
                let line = item_fn.sig.ident.span().start().line as u32;
                let params = rust_param_names(&item_fn.sig.inputs);
                let is_pass_through =
                    if let Some(callee) = rust_pass_through_target(&item_fn, &params) {
                        pass_map.insert(
                            fn_name.clone(),
                            RustPassThrough {
                                callee,
                                line: Some(line),
                            },
                        );
                        true
                    } else {
                        false
                    };

                if !is_pass_through {
                    let fragment = item_fn.block.to_token_stream().to_string();
                    let (normalized, token_count) = normalize_code_for_duplication(&fragment);
                    dup_signatures.push(DupSignature {
                        language: Language::Rust,
                        normalized,
                        token_count,
                        path: path.to_path_buf(),
                        line: Some(line),
                        symbol: Some(fn_name.clone()),
                    });
                }

                let placeholders = rust_placeholders(&item_fn.block, &fn_name);
                for placeholder in placeholders {
                    placeholder_findings.push(FlowFinding {
                        category: FindingCategory::Placeholder,
                        severity: placeholder.severity,
                        confidence: FindingConfidence::High,
                        message: placeholder.message,
                        path: path.to_string_lossy().replace('\\', "/"),
                        line: placeholder.line,
                        symbol: Some(fn_name.clone()),
                        language: Language::Rust,
                        evidence: vec![placeholder.evidence],
                    });
                }
            }
            syn::Item::Impl(item_impl) => {
                let self_ty = item_impl
                    .self_ty
                    .to_token_stream()
                    .to_string()
                    .replace(' ', "");
                for impl_item in item_impl.items {
                    if let syn::ImplItem::Fn(method) = impl_item {
                        let fn_name = method.sig.ident.to_string();
                        let symbol = format!("{self_ty}::{fn_name}");
                        let line = method.sig.ident.span().start().line as u32;
                        let params = rust_param_names(&method.sig.inputs);
                        let is_pass_through = if let Some(callee) =
                            rust_pass_through_target_impl(&method, &params, &self_ty)
                        {
                            pass_map.insert(
                                symbol.clone(),
                                RustPassThrough {
                                    callee,
                                    line: Some(line),
                                },
                            );
                            true
                        } else {
                            false
                        };

                        if !is_pass_through {
                            let fragment = method.block.to_token_stream().to_string();
                            let (normalized, token_count) =
                                normalize_code_for_duplication(&fragment);
                            dup_signatures.push(DupSignature {
                                language: Language::Rust,
                                normalized,
                                token_count,
                                path: path.to_path_buf(),
                                line: Some(line),
                                symbol: Some(symbol.clone()),
                            });
                        }

                        let placeholders = rust_placeholders(&method.block, &symbol);
                        for placeholder in placeholders {
                            placeholder_findings.push(FlowFinding {
                                category: FindingCategory::Placeholder,
                                severity: placeholder.severity,
                                confidence: FindingConfidence::High,
                                message: placeholder.message,
                                path: path.to_string_lossy().replace('\\', "/"),
                                line: placeholder.line,
                                symbol: Some(symbol.clone()),
                                language: Language::Rust,
                                evidence: vec![placeholder.evidence],
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    RustFileReport {
        path: path.to_path_buf(),
        trait_defs,
        trait_impls,
        pass_map,
        placeholder_findings,
        dup_signatures,
    }
}

fn has_allow_marker(attrs: &[syn::Attribute]) -> bool {
    let mut allow = false;
    for attr in attrs {
        if attr.path().is_ident("doc") {
            if let syn::Meta::NameValue(nv) = &attr.meta {
                if let syn::Expr::Lit(expr_lit) = &nv.value {
                    if let syn::Lit::Str(s) = &expr_lit.lit {
                        let text = s.value().to_lowercase();
                        if text.contains("dwg:allow-lonely")
                            || text.contains("dwg:allow-lonely-trait")
                        {
                            allow = true;
                        }
                    }
                }
            }
        }
    }
    allow
}

fn rust_param_names(
    inputs: &syn::punctuated::Punctuated<syn::FnArg, syn::token::Comma>,
) -> Vec<String> {
    let mut names = Vec::new();
    for input in inputs {
        match input {
            syn::FnArg::Receiver(_) => names.push("self".into()),
            syn::FnArg::Typed(pat) => {
                if let syn::Pat::Ident(ident) = &*pat.pat {
                    names.push(ident.ident.to_string());
                }
            }
        }
    }
    names
}

fn rust_pass_through_target(item_fn: &syn::ItemFn, params: &[String]) -> Option<String> {
    rust_pass_through_target_block(&item_fn.block, params, None)
}

fn rust_pass_through_target_impl(
    method: &syn::ImplItemFn,
    params: &[String],
    self_ty: &str,
) -> Option<String> {
    rust_pass_through_target_block(&method.block, params, Some(self_ty))
}

fn rust_pass_through_target_block(
    block: &syn::Block,
    params: &[String],
    self_ty: Option<&str>,
) -> Option<String> {
    let stmt = match block.stmts.len() {
        1 => &block.stmts[0],
        _ => return None,
    };
    let expr = match stmt {
        syn::Stmt::Expr(expr, _) => expr,
        syn::Stmt::Local(_) => return None,
        syn::Stmt::Item(_) => return None,
        syn::Stmt::Macro(_) => return None,
    };

    let mut expr = match expr {
        syn::Expr::Return(ret) => ret.expr.as_ref().map(|e| e.as_ref())?,
        _ => expr,
    };

    loop {
        expr = match expr {
            syn::Expr::Await(await_expr) => await_expr.base.as_ref(),
            syn::Expr::Try(try_expr) => try_expr.expr.as_ref(),
            syn::Expr::Paren(paren) => paren.expr.as_ref(),
            syn::Expr::Group(group) => group.expr.as_ref(),
            _ => break,
        };
    }

    match expr {
        syn::Expr::Call(call) => {
            if !rust_args_match(params, &call.args) {
                return None;
            }
            let syn::Expr::Path(path) = &*call.func else {
                return None;
            };
            let last = path
                .path
                .segments
                .last()
                .map(|seg| seg.ident.to_string())
                .unwrap_or_else(|| "unknown".into());
            if path.path.segments.len() >= 2 && path.path.segments[0].ident == "Self" {
                if let Some(self_ty) = self_ty {
                    return Some(format!("{self_ty}::{last}"));
                }
            }
            Some(last)
        }
        syn::Expr::MethodCall(call) => {
            let mut param_names = params.to_vec();
            if param_names.first().map(|s| s == "self").unwrap_or(false) {
                if is_self_expr(&call.receiver) {
                    param_names.remove(0);
                } else {
                    return None;
                }
            }
            if !rust_args_match(&param_names, &call.args) {
                return None;
            }
            let callee = call.method.to_string();
            if let Some(self_ty) = self_ty {
                Some(format!("{self_ty}::{callee}"))
            } else {
                Some(callee)
            }
        }
        _ => None,
    }
}

fn is_self_expr(expr: &syn::Expr) -> bool {
    matches!(expr, syn::Expr::Path(path) if path.path.is_ident("self"))
}

fn rust_args_match(
    params: &[String],
    args: &syn::punctuated::Punctuated<syn::Expr, syn::token::Comma>,
) -> bool {
    if params.len() != args.len() {
        return false;
    }
    for (param, arg) in params.iter().zip(args.iter()) {
        if !arg_matches_param(param, arg) {
            return false;
        }
    }
    true
}

fn arg_matches_param(param: &str, expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::Path(path) => path.path.is_ident(param),
        syn::Expr::Reference(reference) => arg_matches_param(param, &reference.expr),
        syn::Expr::MethodCall(method) => {
            method.method == "clone" && arg_matches_param(param, &method.receiver)
        }
        _ => false,
    }
}

struct RustPlaceholder {
    severity: FindingSeverity,
    message: String,
    line: Option<u32>,
    evidence: String,
}

fn rust_placeholders(block: &syn::Block, fn_name: &str) -> Vec<RustPlaceholder> {
    use syn::visit::Visit;

    struct PlaceholderVisitor {
        placeholders: Vec<RustPlaceholder>,
        fn_name: String,
    }

    impl<'ast> Visit<'ast> for PlaceholderVisitor {
        fn visit_expr_macro(&mut self, node: &'ast syn::ExprMacro) {
            let name = node
                .mac
                .path
                .segments
                .last()
                .map(|seg| seg.ident.to_string())
                .unwrap_or_default();
            let tokens = node.mac.tokens.to_string().to_lowercase();
            let line = node
                .mac
                .path
                .segments
                .last()
                .map(|seg| seg.ident.span().start().line as u32);
            match name.as_str() {
                "todo" | "unimplemented" => self.placeholders.push(RustPlaceholder {
                    severity: FindingSeverity::Error,
                    message: format!("Placeholder `{}` in `{}`", name, self.fn_name),
                    line,
                    evidence: format!("{}! macro detected", name),
                }),
                "panic" if tokens.contains("todo") || tokens.contains("not implemented") => {
                    self.placeholders.push(RustPlaceholder {
                        severity: FindingSeverity::Warning,
                        message: format!("Panic placeholder in `{}`", self.fn_name),
                        line,
                        evidence: "panic! with TODO/NotImplemented".into(),
                    })
                }
                "unreachable" if tokens.contains("todo") => {
                    self.placeholders.push(RustPlaceholder {
                        severity: FindingSeverity::Warning,
                        message: format!("Unreachable placeholder in `{}`", self.fn_name),
                        line,
                        evidence: "unreachable! with TODO".into(),
                    })
                }
                _ => {}
            }
            syn::visit::visit_expr_macro(self, node);
        }
    }

    let mut visitor = PlaceholderVisitor {
        placeholders: Vec::new(),
        fn_name: fn_name.into(),
    };
    visitor.visit_block(block);
    visitor.placeholders
}

#[derive(Default)]
struct TsAggregate {
    interface_defs: Vec<TsInterfaceInfo>,
    interface_impls: HashMap<String, usize>,
    pass_maps: HashMap<PathBuf, HashMap<String, TsPassThrough>>,
    placeholder_findings: Vec<FlowFinding>,
}

impl TsAggregate {
    fn absorb(&mut self, report: TsFileReport) {
        self.interface_defs.extend(report.interface_defs);
        for name in report.interface_impls {
            *self.interface_impls.entry(name).or_insert(0) += 1;
        }
        if !report.pass_map.is_empty() {
            self.pass_maps.insert(report.path.clone(), report.pass_map);
        }
        self.placeholder_findings
            .extend(report.placeholder_findings);
    }

    fn emit_findings(&self, findings: &mut Vec<FlowFinding>, config: &FlowAuditConfig) {
        for iface in &self.interface_defs {
            let count = self.interface_impls.get(&iface.name).copied().unwrap_or(0);
            if count <= 1 {
                findings.push(FlowFinding {
                    category: FindingCategory::LonelyAbstraction,
                    severity: if count == 0 {
                        FindingSeverity::Warning
                    } else {
                        FindingSeverity::Info
                    },
                    confidence: FindingConfidence::Medium,
                    message: format!(
                        "Interface `{}` has {} implementation{}.",
                        iface.name,
                        count,
                        if count == 1 { "" } else { "s" }
                    ),
                    path: to_display_path(&iface.path, config),
                    line: iface.line,
                    symbol: Some(iface.name.clone()),
                    language: iface.language.clone(),
                    evidence: vec!["Interface has <=1 implements in scan.".into()],
                });
            }
        }

        for (path, pass_map) in &self.pass_maps {
            // Build reverse map: which functions are callees
            let mut reverse: HashMap<String, usize> = HashMap::new();
            for (_, info) in pass_map {
                *reverse.entry(info.callee.clone()).or_insert(0) += 1;
            }
            let mut seen = HashSet::new();
            for (caller, info) in pass_map {
                // Only start chains from functions that are NOT callees of other pass-throughs
                if reverse.contains_key(caller) {
                    continue;
                }
                if seen.contains(caller) {
                    continue;
                }
                let chain = build_pass_chain(caller, pass_map, |info| info.callee.as_str());
                if chain.len() >= 2 {
                    seen.extend(chain.iter().cloned());
                    let chain_msg = chain.join(" -> ");
                    findings.push(FlowFinding {
                        category: FindingCategory::PassThrough,
                        severity: FindingSeverity::Info,
                        confidence: FindingConfidence::Medium,
                        message: format!(
                            "Pass-through wrapper chain length {}: {}",
                            chain.len(),
                            chain_msg
                        ),
                        path: to_display_path(path, config),
                        line: info.line,
                        symbol: Some(caller.clone()),
                        language: info.language.clone(),
                        evidence: vec![format!("Forward-only functions: {}", chain_msg)],
                    });
                }
            }
        }

        for mut finding in self.placeholder_findings.clone() {
            normalize_finding_path(&mut finding, config);
            findings.push(finding);
        }
    }
}

#[derive(Clone)]
struct TsPassThrough {
    callee: String,
    line: Option<u32>,
    language: Language,
}

#[derive(Clone)]
struct TsInterfaceInfo {
    name: String,
    path: PathBuf,
    line: Option<u32>,
    language: Language,
}

struct TsFileReport {
    path: PathBuf,
    interface_defs: Vec<TsInterfaceInfo>,
    interface_impls: Vec<String>,
    pass_map: HashMap<String, TsPassThrough>,
    placeholder_findings: Vec<FlowFinding>,
    dup_signatures: Vec<DupSignature>,
}

fn analyze_ts_file(path: &Path, text: &str, language: &Language) -> TsFileReport {
    use tree_sitter::{Node, Parser};

    let mut parser = Parser::new();
    let lang = if matches!(language, Language::TypeScript) {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT
    } else {
        tree_sitter_javascript::LANGUAGE
    };
    if parser.set_language(&lang.into()).is_err() {
        return TsFileReport {
            path: path.to_path_buf(),
            interface_defs: Vec::new(),
            interface_impls: Vec::new(),
            pass_map: HashMap::new(),
            placeholder_findings: Vec::new(),
            dup_signatures: Vec::new(),
        };
    }

    let tree = match parser.parse(text, None) {
        Some(tree) => tree,
        None => {
            return TsFileReport {
                path: path.to_path_buf(),
                interface_defs: Vec::new(),
                interface_impls: Vec::new(),
                pass_map: HashMap::new(),
                placeholder_findings: Vec::new(),
                dup_signatures: Vec::new(),
            }
        }
    };

    let mut interface_defs = Vec::new();
    let mut interface_impls = Vec::new();
    let mut pass_map = HashMap::new();
    let mut placeholder_findings = Vec::new();
    let mut dup_signatures = Vec::new();

    fn node_text<'a>(node: Node<'a>, src: &'a str) -> String {
        node.utf8_text(src.as_bytes()).unwrap_or("").to_string()
    }

    fn collect_identifiers<'a>(node: Node<'a>, src: &'a str, out: &mut Vec<String>) {
        let kind = node.kind();
        if kind == "identifier" || kind == "property_identifier" || kind == "type_identifier" {
            out.push(node_text(node, src));
        }
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                collect_identifiers(child, src, out);
            }
        }
    }

    fn params_from_node<'a>(params_node: Node<'a>, src: &'a str) -> Option<Vec<String>> {
        let mut names = Vec::new();
        for i in 0..params_node.child_count() {
            if let Some(child) = params_node.child(i) {
                let kind = child.kind();
                if kind == "identifier" {
                    names.push(node_text(child, src));
                } else if kind == "required_parameter"
                    || kind == "optional_parameter"
                    || kind == "rest_parameter"
                {
                    let mut inner = Vec::new();
                    collect_identifiers(child, src, &mut inner);
                    if inner.is_empty() {
                        return None;
                    }
                    names.push(inner[0].clone());
                } else if kind == "object_pattern" || kind == "array_pattern" {
                    return None;
                }
            }
        }
        Some(names)
    }

    fn is_pass_through<'a>(body: Node<'a>, params: &[String], src: &'a str) -> Option<String> {
        let mut statements = Vec::new();
        for i in 0..body.child_count() {
            if let Some(child) = body.child(i) {
                if child.is_named() {
                    statements.push(child);
                }
            }
        }
        if statements.len() != 1 {
            return None;
        }
        let stmt = statements[0];
        if stmt.kind() != "return_statement" {
            return None;
        }
        let call = stmt.child_by_field_name("argument")?;
        if call.kind() != "call_expression" {
            return None;
        }
        let function_node = call.child_by_field_name("function")?;
        let callee = node_text(function_node, src);
        let args_node = call.child_by_field_name("arguments")?;
        let mut args = Vec::new();
        for i in 0..args_node.child_count() {
            if let Some(arg) = args_node.child(i) {
                if arg.kind() == "identifier" {
                    args.push(node_text(arg, src));
                }
            }
        }
        if args.len() != params.len() {
            return None;
        }
        for (param, arg) in params.iter().zip(args.iter()) {
            if param != arg {
                return None;
            }
        }
        Some(callee)
    }

    fn has_placeholder<'a>(body: Node<'a>, src: &'a str) -> bool {
        let mut stack = vec![body];
        while let Some(node) = stack.pop() {
            if node.kind() == "throw_statement" {
                let text = node_text(node, src).to_lowercase();
                if text.contains("todo") || text.contains("not implemented") {
                    return true;
                }
            }
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    stack.push(child);
                }
            }
        }
        false
    }

    let root = tree.root_node();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node.kind() {
            "interface_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = node_text(name_node, text);
                    interface_defs.push(TsInterfaceInfo {
                        name,
                        path: path.to_path_buf(),
                        line: Some((name_node.start_position().row + 1) as u32),
                        language: language.clone(),
                    });
                }
            }
            "class_declaration" => {
                if let Some(impls_node) = node.child_by_field_name("implements") {
                    let mut names = Vec::new();
                    collect_identifiers(impls_node, text, &mut names);
                    for name in names {
                        interface_impls.push(name);
                    }
                }
            }
            "function_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = node_text(name_node, text);
                    let line = Some((name_node.start_position().row + 1) as u32);
                    if let Some(params_node) = node.child_by_field_name("parameters") {
                        if let Some(params) = params_from_node(params_node, text) {
                            if let Some(body) = node.child_by_field_name("body") {
                                match is_pass_through(body, &params, text) {
                                    Some(callee) => {
                                        pass_map.insert(
                                            name.clone(),
                                            TsPassThrough {
                                                callee,
                                                line,
                                                language: language.clone(),
                                            },
                                        );
                                    }
                                    None => {
                                        let fragment = node_text(body, text);
                                        let (normalized, token_count) =
                                            normalize_code_for_duplication(&fragment);
                                        dup_signatures.push(DupSignature {
                                            language: language.clone(),
                                            normalized,
                                            token_count,
                                            path: path.to_path_buf(),
                                            line,
                                            symbol: Some(name.clone()),
                                        });
                                    }
                                }
                                if has_placeholder(body, text) {
                                    placeholder_findings.push(FlowFinding {
                                        category: FindingCategory::Placeholder,
                                        severity: FindingSeverity::Warning,
                                        confidence: FindingConfidence::Medium,
                                        message: format!("Placeholder throw in `{}`", name),
                                        path: path.to_string_lossy().replace('\\', "/"),
                                        line,
                                        symbol: Some(name.clone()),
                                        language: language.clone(),
                                        evidence: vec![
                                            "throw statement with TODO/NotImplemented".into()
                                        ],
                                    });
                                }
                            }
                        }
                    }
                }
            }
            "variable_declarator" => {
                let name_node = node.child_by_field_name("name");
                let value_node = node.child_by_field_name("value");
                if let (Some(name_node), Some(value_node)) = (name_node, value_node) {
                    if value_node.kind() == "arrow_function" || value_node.kind() == "function" {
                        let name = node_text(name_node, text);
                        let line = Some((name_node.start_position().row + 1) as u32);
                        if let Some(params_node) = value_node.child_by_field_name("parameters") {
                            if let Some(params) = params_from_node(params_node, text) {
                                if let Some(body) = value_node.child_by_field_name("body") {
                                    match is_pass_through(body, &params, text) {
                                        Some(callee) => {
                                            pass_map.insert(
                                                name.clone(),
                                                TsPassThrough {
                                                    callee,
                                                    line,
                                                    language: language.clone(),
                                                },
                                            );
                                        }
                                        None => {
                                            let fragment = node_text(body, text);
                                            let (normalized, token_count) =
                                                normalize_code_for_duplication(&fragment);
                                            dup_signatures.push(DupSignature {
                                                language: language.clone(),
                                                normalized,
                                                token_count,
                                                path: path.to_path_buf(),
                                                line,
                                                symbol: Some(name.clone()),
                                            });
                                        }
                                    }
                                    if has_placeholder(body, text) {
                                        placeholder_findings.push(FlowFinding {
                                            category: FindingCategory::Placeholder,
                                            severity: FindingSeverity::Warning,
                                            confidence: FindingConfidence::Medium,
                                            message: format!("Placeholder throw in `{}`", name),
                                            path: path.to_string_lossy().replace('\\', "/"),
                                            line,
                                            symbol: Some(name.clone()),
                                            language: language.clone(),
                                            evidence: vec![
                                                "throw statement with TODO/NotImplemented".into(),
                                            ],
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        if node.child_count() > 0 {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
    }

    TsFileReport {
        path: path.to_path_buf(),
        interface_defs,
        interface_impls,
        pass_map,
        placeholder_findings,
        dup_signatures,
    }
}

#[derive(Default)]
struct PyAggregate {
    abstract_defs: Vec<PyAbstractInfo>,
    subclass_counts: HashMap<String, usize>,
    pass_maps: HashMap<PathBuf, HashMap<String, PyPassThrough>>,
    placeholder_findings: Vec<FlowFinding>,
}

impl PyAggregate {
    fn absorb(&mut self, report: PyFileReport) {
        self.abstract_defs.extend(report.abstract_defs);
        for base in report.subclass_bases {
            *self.subclass_counts.entry(base).or_insert(0) += 1;
        }
        if !report.pass_map.is_empty() {
            self.pass_maps.insert(report.path.clone(), report.pass_map);
        }
        self.placeholder_findings
            .extend(report.placeholder_findings);
    }

    fn emit_findings(&self, findings: &mut Vec<FlowFinding>, config: &FlowAuditConfig) {
        for abs in &self.abstract_defs {
            let count = self.subclass_counts.get(&abs.name).copied().unwrap_or(0);
            if count <= 1 {
                findings.push(FlowFinding {
                    category: FindingCategory::LonelyAbstraction,
                    severity: if count == 0 {
                        FindingSeverity::Warning
                    } else {
                        FindingSeverity::Info
                    },
                    confidence: FindingConfidence::Medium,
                    message: format!(
                        "Abstract base `{}` has {} subclass{}.",
                        abs.name,
                        count,
                        if count == 1 { "" } else { "es" }
                    ),
                    path: to_display_path(&abs.path, config),
                    line: abs.line,
                    symbol: Some(abs.name.clone()),
                    language: Language::Python,
                    evidence: vec!["ABC/Protocol has <=1 subclass in scan.".into()],
                });
            }
        }

        for (path, pass_map) in &self.pass_maps {
            // Build reverse map: which functions are callees
            let mut reverse: HashMap<String, usize> = HashMap::new();
            for (_, info) in pass_map {
                *reverse.entry(info.callee.clone()).or_insert(0) += 1;
            }
            let mut seen = HashSet::new();
            for (caller, info) in pass_map {
                // Only start chains from functions that are NOT callees of other pass-throughs
                if reverse.contains_key(caller) {
                    continue;
                }
                if seen.contains(caller) {
                    continue;
                }
                let chain = build_pass_chain(caller, pass_map, |info| info.callee.as_str());
                if chain.len() >= 2 {
                    seen.extend(chain.iter().cloned());
                    let chain_msg = chain.join(" -> ");
                    findings.push(FlowFinding {
                        category: FindingCategory::PassThrough,
                        severity: FindingSeverity::Info,
                        confidence: FindingConfidence::Medium,
                        message: format!(
                            "Pass-through wrapper chain length {}: {}",
                            chain.len(),
                            chain_msg
                        ),
                        path: to_display_path(path, config),
                        line: info.line,
                        symbol: Some(caller.clone()),
                        language: Language::Python,
                        evidence: vec![format!("Forward-only functions: {}", chain_msg)],
                    });
                }
            }
        }

        for mut finding in self.placeholder_findings.clone() {
            normalize_finding_path(&mut finding, config);
            findings.push(finding);
        }
    }
}

fn normalize_finding_path(finding: &mut FlowFinding, config: &FlowAuditConfig) {
    if let Some(base) = &config.base_dir {
        let path = Path::new(&finding.path);
        if let Ok(rel) = path.strip_prefix(base) {
            finding.path = rel.to_string_lossy().replace('\\', "/");
        }
    }
}

#[derive(Clone)]
struct PyPassThrough {
    callee: String,
    line: Option<u32>,
}

#[derive(Clone)]
struct PyAbstractInfo {
    name: String,
    path: PathBuf,
    line: Option<u32>,
}

struct PyFileReport {
    path: PathBuf,
    abstract_defs: Vec<PyAbstractInfo>,
    subclass_bases: Vec<String>,
    pass_map: HashMap<String, PyPassThrough>,
    placeholder_findings: Vec<FlowFinding>,
    dup_signatures: Vec<DupSignature>,
}

fn analyze_py_file(path: &Path, text: &str) -> PyFileReport {
    use tree_sitter::{Node, Parser};

    let mut parser = Parser::new();
    let lang = tree_sitter_python::LANGUAGE;
    if parser.set_language(&lang.into()).is_err() {
        return PyFileReport {
            path: path.to_path_buf(),
            abstract_defs: Vec::new(),
            subclass_bases: Vec::new(),
            pass_map: HashMap::new(),
            placeholder_findings: Vec::new(),
            dup_signatures: Vec::new(),
        };
    }

    let tree = match parser.parse(text, None) {
        Some(tree) => tree,
        None => {
            return PyFileReport {
                path: path.to_path_buf(),
                abstract_defs: Vec::new(),
                subclass_bases: Vec::new(),
                pass_map: HashMap::new(),
                placeholder_findings: Vec::new(),
                dup_signatures: Vec::new(),
            }
        }
    };

    fn node_text<'a>(node: Node<'a>, src: &'a str) -> String {
        node.utf8_text(src.as_bytes()).unwrap_or("").to_string()
    }

    fn collect_identifiers<'a>(node: Node<'a>, src: &'a str, out: &mut Vec<String>) {
        if node.kind() == "identifier" {
            out.push(node_text(node, src));
        }
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                collect_identifiers(child, src, out);
            }
        }
    }

    fn params_from_node<'a>(params_node: Node<'a>, src: &'a str) -> Option<Vec<String>> {
        let mut names = Vec::new();
        for i in 0..params_node.child_count() {
            if let Some(child) = params_node.child(i) {
                if child.kind() == "identifier" {
                    names.push(node_text(child, src));
                } else if child.kind() == "typed_parameter" || child.kind() == "default_parameter" {
                    let mut inner = Vec::new();
                    collect_identifiers(child, src, &mut inner);
                    if inner.is_empty() {
                        return None;
                    }
                    names.push(inner[0].clone());
                } else if child.kind() == "list_splat" || child.kind() == "dictionary_splat" {
                    return None;
                }
            }
        }
        Some(names)
    }

    fn is_pass_through<'a>(body: Node<'a>, params: &[String], src: &'a str) -> Option<String> {
        let mut statements = Vec::new();
        for i in 0..body.child_count() {
            if let Some(child) = body.child(i) {
                if child.is_named() {
                    statements.push(child);
                }
            }
        }
        if statements.len() != 1 {
            return None;
        }
        let stmt = statements[0];
        if stmt.kind() != "return_statement" {
            return None;
        }
        let call = stmt.child_by_field_name("argument")?;
        if call.kind() != "call" {
            return None;
        }
        let function_node = call.child_by_field_name("function")?;
        let callee = node_text(function_node, src);
        let args_node = call.child_by_field_name("arguments")?;
        let mut args = Vec::new();
        for i in 0..args_node.child_count() {
            if let Some(arg) = args_node.child(i) {
                if arg.kind() == "identifier" {
                    args.push(node_text(arg, src));
                }
            }
        }
        if args.len() != params.len() {
            return None;
        }
        for (param, arg) in params.iter().zip(args.iter()) {
            if param != arg {
                return None;
            }
        }
        Some(callee)
    }

    fn has_placeholder<'a>(body: Node<'a>, src: &'a str) -> bool {
        let mut stack = vec![body];
        while let Some(node) = stack.pop() {
            match node.kind() {
                "pass_statement" => return true,
                "raise_statement" => {
                    let text = node_text(node, src).to_lowercase();
                    if text.contains("notimplementederror") || text.contains("todo") {
                        return true;
                    }
                }
                "expression_statement" => {
                    let text = node_text(node, src).trim().to_string();
                    if text == "..." {
                        return true;
                    }
                }
                _ => {}
            }
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    stack.push(child);
                }
            }
        }
        false
    }

    let root = tree.root_node();
    let mut abstract_defs = Vec::new();
    let mut subclass_bases = Vec::new();
    let mut pass_map = HashMap::new();
    let mut placeholder_findings = Vec::new();
    let mut dup_signatures = Vec::new();

    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "class_definition" {
            let name_node = node.child_by_field_name("name");
            let bases_node = node.child_by_field_name("superclasses");
            let name = name_node.map(|n| node_text(n, text)).unwrap_or_default();
            if let Some(bases_node) = bases_node {
                let mut base_names = Vec::new();
                collect_identifiers(bases_node, text, &mut base_names);
                for base in &base_names {
                    subclass_bases.push(base.clone());
                }
                if base_names.iter().any(|b| b == "ABC" || b == "Protocol") {
                    if let Some(name_node) = name_node {
                        abstract_defs.push(PyAbstractInfo {
                            name: name.clone(),
                            path: path.to_path_buf(),
                            line: Some((name_node.start_position().row + 1) as u32),
                        });
                    }
                }
            }
        }
        if node.kind() == "function_definition" {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, text);
                let line = Some((name_node.start_position().row + 1) as u32);
                if let Some(params_node) = node.child_by_field_name("parameters") {
                    if let Some(params) = params_from_node(params_node, text) {
                        if let Some(body) = node.child_by_field_name("body") {
                            match is_pass_through(body, &params, text) {
                                Some(callee) => {
                                    pass_map.insert(name.clone(), PyPassThrough { callee, line });
                                }
                                None => {
                                    let fragment = node_text(body, text);
                                    let (normalized, token_count) =
                                        normalize_code_for_duplication(&fragment);
                                    dup_signatures.push(DupSignature {
                                        language: Language::Python,
                                        normalized,
                                        token_count,
                                        path: path.to_path_buf(),
                                        line,
                                        symbol: Some(name.clone()),
                                    });
                                }
                            }
                            if has_placeholder(body, text) {
                                placeholder_findings.push(FlowFinding {
                                    category: FindingCategory::Placeholder,
                                    severity: FindingSeverity::Warning,
                                    confidence: FindingConfidence::Medium,
                                    message: format!("Placeholder body in `{}`", name),
                                    path: path.to_string_lossy().replace('\\', "/"),
                                    line,
                                    symbol: Some(name.clone()),
                                    language: Language::Python,
                                    evidence: vec!["pass/ellipsis/NotImplementedError".into()],
                                });
                            }
                        }
                    }
                }
            }
        }
        if node.child_count() > 0 {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
    }

    PyFileReport {
        path: path.to_path_buf(),
        abstract_defs,
        subclass_bases,
        pass_map,
        placeholder_findings,
        dup_signatures,
    }
}
