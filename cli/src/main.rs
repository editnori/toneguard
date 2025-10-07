use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context};
use clap::{ArgAction, Parser};
use console::style;
use dwg_core::{Analyzer, Category, CommentPolicy, Config, DocumentReport};
use globset::{Glob, GlobSet, GlobSetBuilder};
use regex::Regex;
use serde::Serialize;
use serde_yaml::Value as YamlValue;
use walkdir::WalkDir;

/// Deterministic Writing Guard CLI entry point.
#[derive(Debug, Parser)]
#[command(name = "dwg", about = "Lint docs for AI-styled writing patterns.")]
struct Args {
    /// Path to config file (YAML). Defaults to layth-style.yml if present.
    #[arg(long, default_value = "layth-style.yml")]
    config: PathBuf,

    /// Emit JSON output for automation / LSP usage.
    #[arg(long, action = ArgAction::SetTrue)]
    json: bool,

    /// Strict mode: exit non-zero on warnings or higher (density >= warn threshold).
    #[arg(long, action = ArgAction::SetTrue)]
    strict: bool,

    /// Optional output of diagnostics per file.
    #[arg(long, action = ArgAction::SetTrue)]
    quiet: bool,

    /// Files or directories to lint.
    #[arg(value_name = "PATH", default_value = ".", num_args = 0..)]
    paths: Vec<PathBuf>,

    /// Force a specific profile name for all files (overrides glob matching).
    #[arg(long, value_name = "NAME")]
    profile: Option<String>,

    /// Enable only these categories (comma-separated). Implies disabling others.
    #[arg(long, value_delimiter = ',', value_name = "CAT[,CAT]")]
    only: Vec<String>,

    /// Enable additional categories (comma-separated).
    #[arg(long, value_delimiter = ',', value_name = "CAT[,CAT]")]
    enable: Vec<String>,

    /// Disable categories (comma-separated).
    #[arg(long, value_delimiter = ',', value_name = "CAT[,CAT]")]
    disable: Vec<String>,

    /// Set config overrides (repeatable as key=value). Example: --set profile_defaults.min_sentences_per_section=2
    #[arg(long = "set", value_name = "KEY=VALUE", num_args = 0..)]
    sets: Vec<String>,

    /// Skip repo-wide hygiene checks.
    #[arg(long, action = ArgAction::SetTrue)]
    no_repo_checks: bool,

    /// Enable only these repo issue categories.
    #[arg(long = "only-repo", value_delimiter = ',', value_name = "RCAT[,RCAT]")]
    only_repo: Vec<String>,

    /// Enable repo issue categories.
    #[arg(
        long = "enable-repo",
        value_delimiter = ',',
        value_name = "RCAT[,RCAT]"
    )]
    enable_repo: Vec<String>,

    /// Disable repo issue categories.
    #[arg(
        long = "disable-repo",
        value_delimiter = ',',
        value_name = "RCAT[,RCAT]"
    )]
    disable_repo: Vec<String>,
}

#[derive(Debug, Parser)]
#[command(
    name = "dwg comments",
    about = "Inspect or strip code comments across the repo."
)]
struct CommentArgs {
    /// Path to config file (YAML).
    #[arg(long, default_value = "layth-style.yml")]
    config: PathBuf,

    /// Remove comment lines from supported files.
    #[arg(long, action = ArgAction::SetTrue)]
    strip: bool,

    /// Files or directories to scan for comments.
    #[arg(value_name = "PATH", num_args = 0..)]
    paths: Vec<PathBuf>,
}

#[derive(Debug, Serialize)]
struct FileResult {
    path: String,
    word_count: usize,
    density_per_100_words: f32,
    category_counts: BTreeMap<Category, usize>,
    diagnostics: Vec<dwg_core::Diagnostic>,
    profile: String,
}

#[derive(Debug, Serialize, Clone)]
struct RepoIssue {
    category: String,
    message: String,
    path: Option<String>,
}

#[derive(Debug, Serialize)]
struct OutputReport {
    files: Vec<FileResult>,
    total_word_count: usize,
    total_diagnostics: usize,
    density_per_100_words: f32,
    repo_issues: Vec<RepoIssue>,
}

fn main() -> anyhow::Result<()> {
    let argv: Vec<OsString> = env::args_os().collect();
    if argv.len() > 1 && argv[1].as_os_str() == OsStr::new("comments") {
        let mut forwarded = Vec::with_capacity(argv.len() - 1);
        forwarded.push(argv[0].clone());
        forwarded.extend_from_slice(&argv[2..]);
        let comment_args = CommentArgs::parse_from(forwarded);
        run_comments(comment_args)?;
        return Ok(());
    }

    let args = Args::parse();
    run_lint(args)
}

fn run_lint(args: Args) -> anyhow::Result<()> {
    let (mut cfg, config_root) = load_config(&args.config)?;
    apply_overrides(&mut cfg, &args.sets)?;
    let analyzer = Analyzer::new(cfg.clone())?;

    let repo_issues = if args.no_repo_checks {
        Vec::new()
    } else {
        let mut issues = run_repo_checks(&cfg.repo_rules, &config_root, &args.paths)?;
        filter_repo_issues(
            &mut issues,
            &args.only_repo,
            &args.enable_repo,
            &args.disable_repo,
        );
        issues
    };
    if !args.json && !args.quiet && !repo_issues.is_empty() {
        println!("{}", style("Repo checks:").bold());
        for issue in &repo_issues {
            match &issue.path {
                Some(path) => println!("  - {}: {}", style(path).cyan(), issue.message),
                None => println!("  - {}", issue.message),
            }
        }
        println!();
    }

    let file_ignore = build_ignore_set(&cfg.repo_rules.ignore_globs)?;

    let mut files = collect_files(&args.paths, file_ignore.as_ref())?;
    files.sort();

    let mut file_reports = Vec::new();
    let mut total_words = 0usize;
    let mut total_diags = 0usize;
    let mut exit_due_to_threshold = false;

    for path in files {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let rel_path = pathdiff::diff_paths(&path, &config_root).unwrap_or_else(|| path.clone());
        let rel_path_clean = rel_path.to_string_lossy().replace("\\", "/");
        let profile_name = if let Some(force) = &args.profile {
            force.as_str()
        } else {
            analyzer.profile_for_path(&rel_path_clean)
        };
        let mut report = analyzer.analyze_profile_name(&content, profile_name)?;
        filter_diagnostics(&mut report, &args.only, &args.enable, &args.disable)?;
        let density = report.density_per_100_words();
        total_words += report.word_count;
        total_diags += report.diagnostics.len();

        if !args.quiet && !args.json {
            print_human_report(&path, &report, density);
        }

        if density >= cfg.scores.fail_threshold_per_100w as f32 {
            exit_due_to_threshold = true;
        } else if args.strict && density >= cfg.scores.warn_threshold_per_100w as f32 {
            exit_due_to_threshold = true;
        }

        file_reports.push(FileResult {
            path: path.to_string_lossy().to_string(),
            word_count: report.word_count,
            density_per_100_words: density,
            category_counts: report.category_counts.clone(),
            diagnostics: report.diagnostics.clone(),
            profile: report.profile.clone(),
        });
    }

    let overall_density = if total_words == 0 {
        total_diags as f32
    } else {
        (total_diags as f32) * 100.0 / total_words as f32
    };

    let output = OutputReport {
        files: file_reports,
        total_word_count: total_words,
        total_diagnostics: total_diags,
        density_per_100_words: overall_density,
        repo_issues: repo_issues.clone(),
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else if !args.quiet {
        println!(
            "\n{} words, {} diagnostics, density {:.2} per 100 words",
            total_words, total_diags, overall_density
        );
    }

    if exit_due_to_threshold {
        std::process::exit(1);
    }

    Ok(())
}

fn build_ignore_set(patterns: &[String]) -> anyhow::Result<Option<GlobSet>> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern)?);
    }
    Ok(Some(builder.build()?))
}

fn collect_files(paths: &[PathBuf], ignore: Option<&GlobSet>) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            let mut walker = WalkDir::new(path).into_iter();
            while let Some(entry_res) = walker.next() {
                let entry = entry_res?;
                let entry_path = entry.path();
                if let Some(set) = ignore {
                    if set.is_match(entry_path) {
                        if entry.file_type().is_dir() {
                            walker.skip_current_dir();
                        }
                        continue;
                    }
                }
                if entry.file_type().is_dir() {
                    continue;
                }
                if entry.file_type().is_file() && is_supported(entry_path) {
                    files.push(entry_path.to_path_buf());
                }
            }
        } else if path.is_file() && is_supported(path) {
            if let Some(set) = ignore {
                if set.is_match(path) {
                    continue;
                }
            }
            files.push(path.clone());
        }
    }
    Ok(files)
}

fn is_supported(path: &Path) -> bool {
    match path.extension().and_then(|s| s.to_str()) {
        Some(ext) => matches!(
            ext.to_lowercase().as_str(),
            "md" | "markdown" | "mdx" | "txt" | "rst"
        ),
        None => false,
    }
}

fn load_config(path: &PathBuf) -> anyhow::Result<(Config, PathBuf)> {
    if path.exists() {
        let text = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config {}", path.display()))?;
        let value: YamlValue = serde_yaml::from_str(&text)
            .with_context(|| format!("Failed to parse YAML {}", path.display()))?;
        let cfg: Config = serde_yaml::from_value(value)
            .with_context(|| format!("Invalid config structure in {}", path.display()))?;
        let dir = path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| env::current_dir().expect("working dir"));
        Ok((cfg, dir))
    } else {
        Ok((Config::default(), env::current_dir()?))
    }
}

fn parse_category(name: &str) -> Option<Category> {
    let n = name.trim().to_lowercase();
    match n.as_str() {
        "puffery" => Some(Category::Puffery),
        "buzzword" => Some(Category::Buzzword),
        "negative-parallelism" | "negative-parallel" => Some(Category::NegativeParallel),
        "rule-of-three" => Some(Category::RuleOfThree),
        "connector-glut" => Some(Category::ConnectorGlut),
        "template" => Some(Category::Template),
        "weasel" => Some(Category::Weasel),
        "transition" => Some(Category::Transition),
        "marketing" => Some(Category::Marketing),
        "structure" => Some(Category::Structure),
        "call-to-action" | "cta" => Some(Category::CallToAction),
        "sentence-length" => Some(Category::SentenceLength),
        "repetition" => Some(Category::Repetition),
        "cadence" => Some(Category::Cadence),
        "confidence" => Some(Category::Confidence),
        "broad-term" => Some(Category::BroadTerm),
        "tone" => Some(Category::Tone),
        "em-dash" | "emdash" => Some(Category::EmDash),
        "formatting" => Some(Category::Formatting),
        "quote-style" => Some(Category::QuoteStyle),
        _ => None,
    }
}

fn filter_diagnostics(
    report: &mut DocumentReport,
    only: &[String],
    enable: &[String],
    disable: &[String],
) -> anyhow::Result<()> {
    use std::collections::BTreeMap as Map;
    let mut only_set = std::collections::HashSet::new();
    let mut enable_set = std::collections::HashSet::new();
    let mut disable_set = std::collections::HashSet::new();
    for s in only {
        if let Some(cat) = parse_category(s) {
            only_set.insert(cat);
        }
    }
    for s in enable {
        if let Some(cat) = parse_category(s) {
            enable_set.insert(cat);
        }
    }
    for s in disable {
        if let Some(cat) = parse_category(s) {
            disable_set.insert(cat);
        }
    }
    let mut filtered = Vec::new();
    let mut counts: Map<Category, usize> = Map::new();
    for d in &report.diagnostics {
        let cat_opt = parse_category(&format!("{}", d.category));
        let Some(cat) = cat_opt else {
            continue;
        };
        let allowed = if !only_set.is_empty() {
            only_set.contains(&cat)
        } else if !enable_set.is_empty() {
            !disable_set.contains(&cat) || enable_set.contains(&cat)
        } else {
            !disable_set.contains(&cat)
        };
        if allowed {
            filtered.push(d.clone());
            *counts.entry(cat).or_default() += 1;
        }
    }
    report.diagnostics = filtered;
    report.category_counts = counts;
    Ok(())
}

fn apply_overrides(cfg: &mut Config, sets: &[String]) -> anyhow::Result<()> {
    for kv in sets {
        let mut parts = kv.splitn(2, '=');
        let key = parts.next().unwrap_or("").trim();
        let val = parts.next().unwrap_or("").trim();
        if key.is_empty() {
            continue;
        }
        match key {
            "scores.warn_threshold_per_100w" => {
                cfg.scores.warn_threshold_per_100w = val
                    .parse::<u32>()
                    .unwrap_or(cfg.scores.warn_threshold_per_100w);
            }
            "scores.fail_threshold_per_100w" => {
                cfg.scores.fail_threshold_per_100w = val
                    .parse::<u32>()
                    .unwrap_or(cfg.scores.fail_threshold_per_100w);
            }
            "limits.connectors_per_sentence" => {
                cfg.limits.connectors_per_sentence = val
                    .parse::<usize>()
                    .unwrap_or(cfg.limits.connectors_per_sentence);
            }
            "profile_defaults.min_sentences_per_section" => {
                if let Ok(v) = val.parse::<usize>() {
                    cfg.profile_defaults.min_sentences_per_section = Some(v);
                }
            }
            "profile_defaults.min_code_blocks" => {
                if let Ok(v) = val.parse::<usize>() {
                    cfg.profile_defaults.min_code_blocks = Some(v);
                }
            }
            "profile_defaults.enable_triad_slop" => {
                cfg.profile_defaults.enable_triad_slop = matches!(val, "true" | "1" | "yes");
            }
            "quote_style" => {
                cfg.quote_style = if val.eq_ignore_ascii_case("straight") {
                    dwg_core::QuoteStyle::Straight
                } else {
                    dwg_core::QuoteStyle::Any
                };
            }
            "heading_style" => {
                cfg.heading_style = if val.eq_ignore_ascii_case("sentence-case") {
                    dwg_core::HeadingStyle::SentenceCase
                } else if val.eq_ignore_ascii_case("title-case") {
                    dwg_core::HeadingStyle::TitleCase
                } else {
                    dwg_core::HeadingStyle::Any
                };
            }
            _ => {}
        }
    }
    Ok(())
}

fn filter_repo_issues(
    issues: &mut Vec<RepoIssue>,
    only: &[String],
    enable: &[String],
    disable: &[String],
) {
    if only.is_empty() && enable.is_empty() && disable.is_empty() {
        return;
    }
    let mut only_set = std::collections::HashSet::new();
    let mut enable_set = std::collections::HashSet::new();
    let mut disable_set = std::collections::HashSet::new();
    for s in only {
        only_set.insert(s.to_lowercase());
    }
    for s in enable {
        enable_set.insert(s.to_lowercase());
    }
    for s in disable {
        disable_set.insert(s.to_lowercase());
    }
    issues.retain(|iss| {
        let cat = iss.category.to_lowercase();
        let allowed = if !only_set.is_empty() {
            only_set.contains(&cat)
        } else if !enable_set.is_empty() {
            !disable_set.contains(&cat) || enable_set.contains(&cat)
        } else {
            !disable_set.contains(&cat)
        };
        allowed
    });
}
fn run_repo_checks(
    rules: &dwg_core::RepoRules,
    config_root: &Path,
    requested_paths: &[PathBuf],
) -> anyhow::Result<Vec<RepoIssue>> {
    if rules.slop_globs.is_empty()
        && rules.banned_dirs.is_empty()
        && rules.suspicious_filenames.is_empty()
        && !rules.duplicate_lock_check
        && rules.large_json_limit_kb.is_none()
    {
        return Ok(Vec::new());
    }

    let ignore_set = if rules.ignore_globs.is_empty() {
        None
    } else {
        let mut builder = GlobSetBuilder::new();
        for pattern in &rules.ignore_globs {
            builder.add(Glob::new(pattern)?);
        }
        Some(builder.build()?)
    };

    let mut slop_set = None;
    if !rules.slop_globs.is_empty() {
        let mut builder = GlobSetBuilder::new();
        for pattern in &rules.slop_globs {
            builder.add(Glob::new(pattern)?);
        }
        slop_set = Some(builder.build()?);
    }

    let allow_large_set = if rules.allow_large_json_globs.is_empty() {
        None
    } else {
        let mut builder = GlobSetBuilder::new();
        for pattern in &rules.allow_large_json_globs {
            builder.add(Glob::new(pattern)?);
        }
        Some(builder.build()?)
    };

    let mut suspicious_regexes = Vec::new();
    for pattern in &rules.suspicious_filenames {
        suspicious_regexes.push(Regex::new(pattern)?);
    }

    let limit_bytes = rules.large_json_limit_kb.map(|kb| kb * 1024);

    let mut json_set = None;
    if !rules.large_json_globs.is_empty() {
        let mut builder = GlobSetBuilder::new();
        for pattern in &rules.large_json_globs {
            builder.add(Glob::new(pattern)?);
        }
        json_set = Some(builder.build()?);
    }

    let mut bases: BTreeSet<PathBuf> = BTreeSet::new();
    if requested_paths.is_empty() {
        bases.insert(config_root.to_path_buf());
    } else {
        for path in requested_paths {
            let resolved = if path.is_dir() {
                path.clone()
            } else {
                path.parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| config_root.to_path_buf())
            };
            bases.insert(resolved);
        }
        if bases.is_empty() {
            bases.insert(config_root.to_path_buf());
        }
    }

    let mut issues = Vec::new();
    for base in &bases {
        let mut walker = WalkDir::new(&base).into_iter();
        while let Some(entry_res) = walker.next() {
            let entry = match entry_res {
                Ok(e) => e,
                Err(err) => {
                    issues.push(RepoIssue {
                        category: "walk".into(),
                        message: format!("Filesystem scan error: {}", err),
                        path: None,
                    });
                    continue;
                }
            };

            let rel = entry
                .path()
                .strip_prefix(config_root)
                .unwrap_or_else(|_| entry.path());

            if ignore_set
                .as_ref()
                .map(|set| set.is_match(rel))
                .unwrap_or(false)
            {
                if entry.file_type().is_dir() {
                    walker.skip_current_dir();
                }
                continue;
            }

            let rel_display = rel.to_string_lossy().replace('\\', "/");

            if entry.file_type().is_dir() {
                let name = entry
                    .file_name()
                    .to_str()
                    .unwrap_or_default()
                    .to_lowercase();
                if rules
                    .banned_dirs
                    .iter()
                    .any(|banned| name == banned.to_lowercase())
                {
                    issues.push(RepoIssue {
                        category: "banned-dir".into(),
                        message: format!("Banned directory `{}` present", rel_display),
                        path: Some(rel_display.clone()),
                    });
                    walker.skip_current_dir();
                    continue;
                }
                continue;
            }

            let file_name = entry.file_name().to_string_lossy();

            if rules
                .banned_dirs
                .iter()
                .any(|banned| file_name.eq_ignore_ascii_case(banned))
            {
                issues.push(RepoIssue {
                    category: "banned-file".into(),
                    message: format!("Banned file `{}` present", rel_display),
                    path: Some(rel_display.clone()),
                });
            }

            if let Some(set) = &slop_set {
                if set.is_match(rel) {
                    issues.push(RepoIssue {
                        category: "slop".into(),
                        message: format!("Suspicious slop path `{}`", rel_display),
                        path: Some(rel_display.clone()),
                    });
                }
            }

            for regex in &suspicious_regexes {
                if regex.is_match(&file_name) {
                    issues.push(RepoIssue {
                        category: "suspicious-name".into(),
                        message: format!("Suspicious filename `{}`", rel_display),
                        path: Some(rel_display.clone()),
                    });
                    break;
                }
            }

            if rules.duplicate_lock_check && file_name == "package-lock.json" {
                let sibling = entry.path().with_file_name("yarn.lock");
                if sibling.exists() {
                    let sibling_rel = sibling
                        .strip_prefix(config_root)
                        .unwrap_or(&sibling)
                        .to_string_lossy()
                        .replace('\\', "/");
                    issues.push(RepoIssue {
                        category: "lockfile".into(),
                        message: format!(
                            "package-lock.json and yarn.lock both present (`{}` vs `{}`)",
                            rel_display, sibling_rel
                        ),
                        path: Some(rel_display.clone()),
                    });
                }
            }

            if let (Some(limit), Some(set)) = (limit_bytes, &json_set) {
                if set.is_match(rel) {
                    let allowed = allow_large_set
                        .as_ref()
                        .map(|allow| allow.is_match(rel))
                        .unwrap_or(false);
                    if !allowed {
                        if let Ok(metadata) = entry.metadata() {
                            if metadata.len() > limit {
                                issues.push(RepoIssue {
                                    category: "large-json".into(),
                                    message: format!(
                                        "Large structured file `{}` ({} KB)",
                                        rel_display,
                                        metadata.len() / 1024
                                    ),
                                    path: Some(rel_display.clone()),
                                });
                            }
                        }
                    }
                }
            }

            // Root-level stray markdown detection
            if rel.components().count() == 1 {
                if let Some(ext) = rel.extension().and_then(|e| e.to_str()) {
                    if ext.eq_ignore_ascii_case("md") {
                        let fname = file_name.to_lowercase();
                        let allowed = fname == "readme.md"
                            || fname.starts_with("license")
                            || fname == "contributing.md"
                            || fname == "code_of_conduct.md"
                            || fname == "security.md"
                            || fname == "changelog.md";
                        let looks_ai = fname.contains("claude")
                            || fname.contains("chatgpt")
                            || fname.contains("copilot")
                            || fname.contains("ai")
                            || fname.contains("agent");
                        let looks_temp = fname.contains("changes")
                            || fname.contains("diff")
                            || fname.contains("output")
                            || fname.contains("response")
                            || fname.contains("analysis")
                            || fname.contains("notes");
                        if !allowed && (looks_ai || looks_temp) {
                            issues.push(RepoIssue {
                                category: "stray-markdown".into(),
                                message: format!("Stray markdown at repo root: `{}`", rel_display),
                                path: Some(rel_display.clone()),
                            });
                        }
                    }
                }
            }
        }
    }

    // Duplicate variant detector per directory (copy/final/draft variants)
    let mut per_dir: std::collections::HashMap<
        String,
        std::collections::HashMap<String, Vec<String>>,
    > = std::collections::HashMap::new();
    for issue in issues.clone() {
        let _ = issue;
    } // keep borrow checker calm for future reuse
    for base in &bases {
        let mut walker = WalkDir::new(&base).into_iter();
        while let Some(entry_res) = walker.next() {
            let entry = match entry_res {
                Ok(e) => e,
                Err(_) => continue,
            };
            let rel = entry
                .path()
                .strip_prefix(config_root)
                .unwrap_or_else(|_| entry.path());
            if ignore_set
                .as_ref()
                .map(|set| set.is_match(rel))
                .unwrap_or(false)
            {
                if entry.file_type().is_dir() {
                    walker.skip_current_dir();
                }
                continue;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let dir = rel
                .parent()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|| String::from("."));
            let fname = entry.file_name().to_string_lossy().to_string();
            let stem = entry
                .path()
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            let ext = entry
                .path()
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            let canonical = canonicalize_stem(&stem);
            per_dir
                .entry(dir)
                .or_default()
                .entry(format!("{}.{}", canonical, ext))
                .or_default()
                .push(fname);
        }
    }
    for (dir, groups) in per_dir {
        for (canon, files) in groups {
            if files.len() > 1 {
                let path_display = if dir == "." {
                    canon.clone()
                } else {
                    format!("{}/{}", dir, canon)
                };
                issues.push(RepoIssue {
                    category: "dup-variants".into(),
                    message: format!(
                        "Multiple variant files detected: {} → {:?}",
                        path_display, files
                    ),
                    path: Some(path_display),
                });
            }
        }
    }

    Ok(issues)
}

fn canonicalize_stem(stem: &str) -> String {
    let mut s = stem.to_string();
    s = s.replace(&[' ', '-', '_'][..], " ");
    let patterns = [
        " copy",
        " backup",
        " old",
        " new",
        " final",
        " final final",
        " final2",
        " draft",
    ];
    for p in patterns {
        if let Some(pos) = s.rfind(p) {
            s.truncate(pos);
        }
    }
    // remove trailing (n)
    let re = Regex::new(r"\s*\(\d+\)$").unwrap();
    s = re.replace(&s, "").to_string();
    s.trim().to_lowercase().replace(' ', "-")
}

fn print_human_report(path: &PathBuf, report: &DocumentReport, density: f32) {
    println!(
        "{} ({} words, density {:.2}/100w, profile {})",
        style(path.to_string_lossy()).bold(),
        report.word_count,
        density,
        report.profile
    );
    if report.diagnostics.is_empty() {
        println!("  {}", style("clean").green());
        return;
    }
    for diag in &report.diagnostics {
        println!(
            "  [{}] {}:{} {}",
            style(diag.category).yellow(),
            diag.location.line,
            diag.location.column,
            diag.message
        );
        if !diag.snippet.is_empty() {
            println!("      → {}", diag.snippet);
        }
        if let Some(suggestion) = &diag.suggestion {
            println!("      suggestion: {}", suggestion);
        }
    }
}

fn run_comments(args: CommentArgs) -> anyhow::Result<()> {
    let (cfg, _) = load_config(&args.config)?;
    let policy = cfg.comment_policy.clone();
    if !policy.enabled {
        eprintln!("Comment policy is disabled in config; running with heuristic defaults.");
    }

    let files = collect_comment_files(&args.paths, &policy)?;
    if files.is_empty() {
        println!("No files matched comment analysis.");
        return Ok(());
    }

    let mut violations = Vec::new();
    for path in files {
        match analyze_comment_stats(&path)? {
            Some(stats) => {
                let ratio = stats.comment_ratio();
                let exceeds = policy.max_ratio.map(|limit| ratio > limit).unwrap_or(false);
                println!(
                    "{} → {} comment lines / {} ({:.1}%){}",
                    style(path.to_string_lossy()).bold(),
                    stats.comment_lines,
                    stats.total_lines,
                    ratio * 100.0,
                    if exceeds { " [exceeds]" } else { "" }
                );
                if exceeds {
                    violations.push(stats);
                } else if args.strip {
                    // optional cleanup even if within limits when explicitly requested
                    violations.push(stats);
                }
            }
            None => {}
        }
    }

    if args.strip {
        for stats in &violations {
            let syntax = stats.syntax;
            if strip_comments(&stats.path, syntax)? {
                println!("  stripped comments from {}", stats.path.display());
            }
        }
    }

    if !violations.is_empty() && !args.strip {
        std::process::exit(2);
    }

    Ok(())
}

fn collect_comment_files(
    paths: &[PathBuf],
    policy: &CommentPolicy,
) -> anyhow::Result<Vec<PathBuf>> {
    let base_paths = if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths.to_vec()
    };

    let allow_globs = build_glob_set(&policy.allow_globs)?;
    let ignore_globs = build_glob_set(&policy.ignore_globs)?;

    let mut files = Vec::new();
    for path in base_paths {
        if path.is_dir() {
            for entry in WalkDir::new(&path) {
                let entry = entry?;
                if !entry.file_type().is_file() {
                    continue;
                }
                let fp = entry.path();
                if let Some(glob) = &ignore_globs {
                    if glob.is_match(fp) {
                        continue;
                    }
                }
                if let Some(glob) = &allow_globs {
                    if !glob.is_match(fp) {
                        continue;
                    }
                }
                if comment_syntax_for(fp).is_none() {
                    continue;
                }
                files.push(fp.to_path_buf());
            }
        } else if path.is_file() {
            if let Some(glob) = &ignore_globs {
                if glob.is_match(&path) {
                    continue;
                }
            }
            if allow_globs
                .as_ref()
                .map(|set| set.is_match(&path))
                .unwrap_or(true)
                && comment_syntax_for(&path).is_some()
            {
                files.push(path.clone());
            }
        }
    }

    Ok(files)
}

fn build_glob_set(patterns: &[String]) -> anyhow::Result<Option<GlobSet>> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern).map_err(|e| anyhow!("invalid glob `{}`: {e}", pattern))?);
    }
    Ok(Some(
        builder
            .build()
            .map_err(|e| anyhow!("failed to build glob set: {e}"))?,
    ))
}

#[derive(Clone, Copy)]
struct CommentSyntax {
    line: Option<&'static str>,
    block_start: Option<&'static str>,
    block_end: Option<&'static str>,
}

impl CommentSyntax {
    fn supports_stripping(&self) -> bool {
        self.block_start.is_none()
    }
}

fn comment_syntax_for(path: &Path) -> Option<CommentSyntax> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let data = match ext.as_str() {
        "rs" | "ts" | "tsx" | "js" | "jsx" | "java" | "go" | "c" | "h" | "cpp" | "hpp" | "cs"
        | "swift" | "kt" | "php" => CommentSyntax {
            line: Some("//"),
            block_start: Some("/*"),
            block_end: Some("*/"),
        },
        "py" | "rb" | "sh" | "bash" | "toml" | "yaml" | "yml" => CommentSyntax {
            line: Some("#"),
            block_start: None,
            block_end: None,
        },
        "sql" => CommentSyntax {
            line: Some("--"),
            block_start: Some("/*"),
            block_end: Some("*/"),
        },
        _ => return None,
    };
    Some(data)
}

struct CommentStats {
    path: PathBuf,
    total_lines: usize,
    comment_lines: usize,
    syntax: CommentSyntax,
}

impl CommentStats {
    fn comment_ratio(&self) -> f32 {
        if self.total_lines == 0 {
            0.0
        } else {
            self.comment_lines as f32 / self.total_lines as f32
        }
    }
}

fn analyze_comment_stats(path: &Path) -> anyhow::Result<Option<CommentStats>> {
    let syntax = match comment_syntax_for(path) {
        Some(s) => s,
        None => return Ok(None),
    };
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(err) => {
            eprintln!("Skipping {} ({err})", path.display());
            return Ok(None);
        }
    };

    let mut total = 0usize;
    let mut comment = 0usize;
    let mut in_block = false;

    for raw_line in content.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }
        total += 1;

        if in_block {
            comment += 1;
            if let Some(end) = syntax.block_end {
                if trimmed.contains(end) {
                    in_block = false;
                }
            } else {
                in_block = false;
            }
            continue;
        }

        if let Some(line_marker) = syntax.line {
            if trimmed.starts_with(line_marker) {
                comment += 1;
                continue;
            }
        }

        if let (Some(start), Some(end)) = (syntax.block_start, syntax.block_end) {
            if trimmed.starts_with(start) {
                comment += 1;
                if !trimmed.contains(end) {
                    in_block = true;
                }
            }
        }
    }

    if total == 0 {
        return Ok(None);
    }

    Ok(Some(CommentStats {
        path: path.to_path_buf(),
        total_lines: total,
        comment_lines: comment,
        syntax,
    }))
}

fn strip_comments(path: &Path, syntax: CommentSyntax) -> anyhow::Result<bool> {
    if !syntax.supports_stripping() {
        eprintln!(
            "Skipping strip for {} (block comments not yet supported)",
            path.display()
        );
        return Ok(false);
    }
    let content =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
    let mut changed = false;
    let mut output = String::new();

    for line in content.lines() {
        let trimmed = line.trim_start();
        if let Some(marker) = syntax.line {
            if trimmed.starts_with(marker) {
                changed = true;
                continue;
            }
        }
        output.push_str(line);
        output.push('\n');
    }

    if changed {
        fs::write(path, output)?;
    }
    Ok(changed)
}
