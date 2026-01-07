use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context};
use clap::{ArgAction, Parser, Subcommand};
use console::style;
use dwg_core::{
    arch::{FlowAuditConfig, FlowAuditReport, Language as FlowLanguage},
    blueprint::{blueprint_paths, BlueprintConfig, BlueprintReport},
    flow::{FlowSpecIssue, IssueSeverity},
    organize::{analyze_organization, generate_organize_prompt, OrganizationReport},
    parse_category, Analyzer, Category, CommentPolicy, Config, DocumentReport,
};
use globset::{Glob, GlobSet, GlobSetBuilder};
use regex::Regex;
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Parser)]
#[command(
    name = "dwg calibrate",
    about = "Learn from good writing samples to tune ToneGuard thresholds."
)]
struct CalibrateArgs {
    /// Path to config file (YAML).
    #[arg(long, default_value = "layth-style.yml")]
    config: PathBuf,

    /// Output file for calibration profile.
    #[arg(long, short, default_value = "calibration.yml")]
    output: PathBuf,

    /// Files or directories of good writing samples to learn from.
    #[arg(value_name = "PATH", num_args = 1..)]
    paths: Vec<PathBuf>,
}

#[derive(Debug, Parser)]
#[command(
    name = "dwg flow",
    about = "Logic flow guardrails: validate flow specs and audit code entropy."
)]
struct FlowArgs {
    #[command(subcommand)]
    command: FlowCommand,
}

#[derive(Debug, Parser)]
#[command(
    name = "dwg organize",
    about = "Analyze repo organization and suggest cleanup."
)]
struct OrganizeArgs {
    /// Path to config file (YAML).
    #[arg(long, default_value = "layth-style.yml")]
    config: PathBuf,

    /// Emit JSON output for automation.
    #[arg(long, action = ArgAction::SetTrue)]
    json: bool,

    /// Generate AI prompt for reorganization (cursor, claude, codex).
    #[arg(long, value_name = "AGENT")]
    prompt_for: Option<String>,

    /// Minimum file size (KB) to flag as data file.
    #[arg(long)]
    data_min_kb: Option<u64>,

    /// Skip git status checks.
    #[arg(long, action = ArgAction::SetTrue)]
    no_git: bool,

    /// Write output to file.
    #[arg(long)]
    out: Option<PathBuf>,

    /// Paths to analyze.
    #[arg(value_name = "PATH", default_value = ".")]
    paths: Vec<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum FlowCommand {
    /// Validate flow specs and invariants.
    Check(FlowCheckArgs),
    /// Run static entropy detectors and optional flow checks.
    Audit(FlowAuditArgs),
    /// Generate a reviewable Markdown artifact from flow checks + audit.
    Propose(FlowProposeArgs),
    /// Create a new flow spec file (an artifact to review).
    New(FlowNewArgs),
    /// Build a repo-wide blueprint graph (files + edges).
    Blueprint(FlowBlueprintArgs),
    /// Index functions and methods across the repo.
    Index(FlowIndexArgs),
    /// Build a cross-file call graph for functions and methods.
    Callgraph(FlowCallgraphArgs),
    /// Generate a control flow graph (CFG) for a function.
    Graph(FlowGraphArgs),
}

#[derive(Debug, Parser)]
struct FlowCheckArgs {
    /// Path to config file (YAML).
    #[arg(long, default_value = "layth-style.yml")]
    config: PathBuf,

    /// Directory containing flow specs.
    #[arg(long, default_value = "flows")]
    flows: PathBuf,

    /// Emit JSON output for automation.
    #[arg(long, action = ArgAction::SetTrue)]
    json: bool,

    /// Write JSON output to file.
    #[arg(long)]
    out: Option<PathBuf>,
}

#[derive(Debug, Parser)]
struct FlowAuditArgs {
    /// Path to config file (YAML).
    #[arg(long, default_value = "layth-style.yml")]
    config: PathBuf,

    /// Directory containing flow specs.
    #[arg(long, default_value = "flows")]
    flows: PathBuf,

    /// Skip flow spec validation.
    #[arg(long, action = ArgAction::SetTrue)]
    no_flow_checks: bool,

    /// Restrict to specific languages (comma-separated: rust,typescript,javascript,python).
    #[arg(long, value_delimiter = ',', value_name = "LANG[,LANG]")]
    language: Vec<String>,

    /// Emit JSON output for automation.
    #[arg(long, action = ArgAction::SetTrue)]
    json: bool,

    /// Write JSON output to file.
    #[arg(long)]
    out: Option<PathBuf>,

    /// Paths to scan (defaults to current directory).
    #[arg(value_name = "PATH", num_args = 0..)]
    paths: Vec<PathBuf>,
}

#[derive(Debug, Parser)]
struct FlowProposeArgs {
    /// Path to config file (YAML).
    #[arg(long, default_value = "layth-style.yml")]
    config: PathBuf,

    /// Directory containing flow specs.
    #[arg(long, default_value = "flows")]
    flows: PathBuf,

    /// Skip flow spec validation.
    #[arg(long, action = ArgAction::SetTrue)]
    no_flow_checks: bool,

    /// Restrict to specific languages (comma-separated: rust,typescript,javascript,python).
    #[arg(long, value_delimiter = ',', value_name = "LANG[,LANG]")]
    language: Vec<String>,

    /// Write Markdown output to file (prints to stdout if omitted).
    #[arg(long)]
    out: Option<PathBuf>,

    /// Paths to scan (defaults to current directory).
    #[arg(value_name = "PATH", num_args = 0..)]
    paths: Vec<PathBuf>,
}

#[derive(Debug, Parser)]
struct FlowNewArgs {
    /// Path to config file (YAML).
    #[arg(long, default_value = "layth-style.yml")]
    config: PathBuf,

    /// Directory containing flow specs.
    #[arg(long, default_value = "flows")]
    flows: PathBuf,

    /// Flow name (human-readable).
    #[arg(long)]
    name: String,

    /// Flow entrypoint (route/command/job/etc).
    #[arg(long)]
    entrypoint: String,

    /// Optional language hint (e.g., rust, typescript, python).
    #[arg(long)]
    language: Option<String>,

    /// Output path (defaults to flows/<slug>.md).
    #[arg(long)]
    out: Option<PathBuf>,

    /// Overwrite if the file already exists.
    #[arg(long, action = ArgAction::SetTrue)]
    force: bool,
}

#[derive(Debug, Parser)]
struct FlowGraphArgs {
    /// Source file to analyze.
    #[arg(long)]
    file: PathBuf,

    /// Function name to graph (optional - if not provided, graphs all functions).
    #[arg(long, value_name = "NAME")]
    r#fn: Option<String>,

    /// Output format: json, mermaid.
    #[arg(long, default_value = "json")]
    format: String,

    /// Write output to file.
    #[arg(long)]
    out: Option<PathBuf>,

    /// Include logic detector findings in output.
    #[arg(long, action = ArgAction::SetTrue)]
    with_logic: bool,

    /// Include Mermaid diagram text in JSON output (adds a `mermaid` field per CFG).
    ///
    /// Note: `--format mermaid` outputs Markdown (not JSON).
    #[arg(long, action = ArgAction::SetTrue)]
    include_mermaid: bool,
}

#[derive(Debug, Parser)]
struct FlowIndexArgs {
    /// Path to config file (YAML).
    #[arg(long, default_value = "layth-style.yml")]
    config: PathBuf,

    /// Write JSON output to file.
    #[arg(long)]
    out: Option<PathBuf>,

    /// Files or directories to scan.
    #[arg(value_name = "PATH", default_value = ".", num_args = 0..)]
    paths: Vec<PathBuf>,
}

#[derive(Debug, Parser)]
struct FlowCallgraphArgs {
    /// Path to config file (YAML).
    #[arg(long, default_value = "layth-style.yml")]
    config: PathBuf,

    /// Output format: json, jsonl.
    #[arg(long, default_value = "json")]
    format: String,

    /// Write output to file.
    #[arg(long)]
    out: Option<PathBuf>,

    /// Maximum calls captured per function body.
    #[arg(long, default_value = "200")]
    max_calls_per_fn: usize,

    /// Emit only resolved call edges (drops unresolved calls).
    #[arg(long, action = ArgAction::SetTrue)]
    resolved_only: bool,

    /// Files or directories to scan.
    #[arg(value_name = "PATH", default_value = ".", num_args = 0..)]
    paths: Vec<PathBuf>,
}

#[derive(Debug, Parser)]
struct FlowBlueprintArgs {
    /// Path to config file (YAML).
    #[arg(long, default_value = "layth-style.yml")]
    config: PathBuf,

    /// Output format: json, jsonl.
    #[arg(long, default_value = "json")]
    format: String,

    /// Write output to file.
    #[arg(long)]
    out: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<FlowBlueprintCommand>,

    /// Files or directories to scan.
    #[arg(value_name = "PATH", default_value = ".", num_args = 0..)]
    paths: Vec<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum FlowBlueprintCommand {
    /// Compare two blueprint snapshots and emit a diff + mapping template.
    Diff(FlowBlueprintDiffArgs),
}

#[derive(Debug, Parser)]
struct FlowBlueprintDiffArgs {
    /// "Before" blueprint snapshot (JSON output from `dwg flow blueprint --format json`).
    #[arg(long)]
    before: PathBuf,

    /// "After" blueprint snapshot (JSON output from `dwg flow blueprint --format json`).
    #[arg(long)]
    after: PathBuf,

    /// Write output to file (prints to stdout if omitted).
    #[arg(long)]
    out: Option<PathBuf>,

    /// Emit Markdown instead of JSON.
    #[arg(long, action = ArgAction::SetTrue)]
    md: bool,

    /// Write a YAML mapping template for removed files.
    #[arg(long = "write-mapping")]
    write_mapping: Option<PathBuf>,

    /// Require a YAML mapping file to cover all removed files.
    #[arg(long = "require-mapping")]
    require_mapping: Option<PathBuf>,

    /// Maximum rename candidates per removed file.
    #[arg(long, default_value = "3")]
    max_candidates: usize,
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

#[derive(Debug, Serialize)]
struct FlowCheckFile {
    path: String,
    issues: Vec<FlowSpecIssue>,
}

#[derive(Debug, Serialize)]
struct FlowCheckReport {
    files: Vec<FlowCheckFile>,
    error_count: usize,
    warning_count: usize,
}

#[derive(Debug, Serialize)]
struct FlowAuditOutput {
    flow_check: Option<FlowCheckReport>,
    audit: FlowAuditReport,
}

fn main() -> anyhow::Result<()> {
    let argv: Vec<OsString> = env::args_os().collect();

    // Handle subcommands
    if argv.len() > 1 {
        let subcommand = argv[1].as_os_str();
        if subcommand == OsStr::new("comments") {
            let mut forwarded = Vec::with_capacity(argv.len() - 1);
            forwarded.push(argv[0].clone());
            forwarded.extend_from_slice(&argv[2..]);
            let comment_args = CommentArgs::parse_from(forwarded);
            run_comments(comment_args)?;
            return Ok(());
        }
        if subcommand == OsStr::new("calibrate") {
            let mut forwarded = Vec::with_capacity(argv.len() - 1);
            forwarded.push(argv[0].clone());
            forwarded.extend_from_slice(&argv[2..]);
            let calibrate_args = CalibrateArgs::parse_from(forwarded);
            run_calibrate(calibrate_args)?;
            return Ok(());
        }
        if subcommand == OsStr::new("flow") {
            let mut forwarded = Vec::with_capacity(argv.len() - 1);
            forwarded.push(argv[0].clone());
            forwarded.extend_from_slice(&argv[2..]);
            let flow_args = FlowArgs::parse_from(forwarded);
            run_flow(flow_args)?;
            return Ok(());
        }
        if subcommand == OsStr::new("organize") {
            let mut forwarded = Vec::with_capacity(argv.len() - 1);
            forwarded.push(argv[0].clone());
            forwarded.extend_from_slice(&argv[2..]);
            let organize_args = OrganizeArgs::parse_from(forwarded);
            run_organize(organize_args)?;
            return Ok(());
        }
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
        let bytes =
            fs::read(&path).with_context(|| format!("Failed to read {}", path.display()))?;
        let content = String::from_utf8_lossy(&bytes).to_string();
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

fn collect_code_files(paths: &[PathBuf], ignore: Option<&GlobSet>) -> anyhow::Result<Vec<PathBuf>> {
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
                if entry.file_type().is_file() && is_supported_code(entry_path) {
                    files.push(entry_path.to_path_buf());
                }
            }
        } else if path.is_file() && is_supported_code(path) {
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

fn is_supported_code(path: &Path) -> bool {
    match path.extension().and_then(|s| s.to_str()) {
        Some(ext) => matches!(
            ext.to_lowercase().as_str(),
            "rs" | "ts" | "tsx" | "js" | "jsx" | "py"
        ),
        None => false,
    }
}

fn load_config(path: &PathBuf) -> anyhow::Result<(Config, PathBuf)> {
    let cwd = env::current_dir()?;
    if path.exists() {
        let text = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config {}", path.display()))?;
        let value: YamlValue = serde_yaml::from_str(&text)
            .with_context(|| format!("Failed to parse YAML {}", path.display()))?;
        let cfg: Config = serde_yaml::from_value(value)
            .with_context(|| format!("Invalid config structure in {}", path.display()))?;

        // Make the config root stable (absolute) so downstream features can produce consistent
        // relative paths (e.g. blueprint edges that resolve to scanned nodes).
        let abs_path = path.canonicalize().unwrap_or_else(|_| {
            if path.is_absolute() {
                path.clone()
            } else {
                cwd.join(path)
            }
        });

        let dir = abs_path.parent().map(|p| p.to_path_buf()).unwrap_or(cwd);
        Ok((cfg, dir))
    } else {
        Ok((Config::default(), cwd))
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
                match path.parent() {
                    Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
                    _ => config_root.to_path_buf(),
                }
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
                            || fname == "changelog.md"
                            || fname == "agents.md";
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

/// Calibrate ToneGuard by learning from good writing samples.
/// Generates a calibration.yml with adjusted thresholds.
fn run_calibrate(args: CalibrateArgs) -> anyhow::Result<()> {
    use std::collections::HashMap;

    let (cfg, _config_root) = load_config(&args.config)?;
    let analyzer = Analyzer::new(cfg.clone())?;

    let file_ignore = build_ignore_set(&cfg.repo_rules.ignore_globs)?;
    let mut files = collect_files(&args.paths, file_ignore.as_ref())?;
    files.sort();

    if files.is_empty() {
        return Err(anyhow!("No markdown files found in the provided paths."));
    }

    println!(
        "{}",
        style(format!("Calibrating from {} files...", files.len())).bold()
    );

    // Collect statistics
    let mut total_words = 0usize;
    let mut phrase_counts: HashMap<String, usize> = HashMap::new();
    let mut category_counts: BTreeMap<Category, usize> = BTreeMap::new();
    let mut sentence_lengths: Vec<usize> = Vec::new();
    let mut file_densities: Vec<f32> = Vec::new();

    for path in &files {
        let bytes = fs::read(path).with_context(|| format!("Failed to read {}", path.display()))?;
        let content = String::from_utf8_lossy(&bytes).to_string();
        let report = analyzer.analyze(&content);

        total_words += report.word_count;
        file_densities.push(report.density_per_100_words());

        // Count category occurrences
        for (cat, count) in &report.category_counts {
            *category_counts.entry(*cat).or_default() += count;
        }

        // Collect flagged phrases to potentially whitelist
        for diag in &report.diagnostics {
            let snippet_lower = diag.snippet.to_lowercase();
            *phrase_counts.entry(snippet_lower).or_default() += 1;
        }

        // Estimate sentence lengths from content
        for sentence in content.split(|c| c == '.' || c == '!' || c == '?') {
            let word_count = sentence.split_whitespace().count();
            if word_count >= 3 {
                sentence_lengths.push(word_count);
            }
        }
    }

    // Calculate statistics
    let avg_density = if file_densities.is_empty() {
        0.0
    } else {
        file_densities.iter().sum::<f32>() / file_densities.len() as f32
    };

    let max_density = file_densities.iter().copied().fold(0.0f32, |a, b| a.max(b));

    let avg_sentence_length = if sentence_lengths.is_empty() {
        20.0
    } else {
        sentence_lengths.iter().sum::<usize>() as f64 / sentence_lengths.len() as f64
    };

    // Identify phrases that appear frequently (candidates for whitelisting)
    let mut whitelist_candidates: Vec<_> = phrase_counts
        .iter()
        .filter(|(_, count)| **count >= 2) // Appears in at least 2 files
        .map(|(phrase, count)| (phrase.clone(), *count))
        .collect();
    whitelist_candidates.sort_by(|a, b| b.1.cmp(&a.1));

    // Generate calibration output
    println!();
    println!("{}", style("Calibration Results:").bold().green());
    println!("  Total files analyzed: {}", files.len());
    println!("  Total words: {}", total_words);
    println!("  Average density: {:.2} flags per 100 words", avg_density);
    println!("  Maximum density: {:.2} flags per 100 words", max_density);
    println!(
        "  Average sentence length: {:.1} words",
        avg_sentence_length
    );
    println!();

    // Suggest thresholds
    let suggested_warn = (max_density * 1.5).max(3.0).round() as u32;
    let suggested_fail = (max_density * 2.5).max(6.0).round() as u32;

    println!("{}", style("Suggested Thresholds:").bold());
    println!("  warn_threshold_per_100w: {}", suggested_warn);
    println!("  fail_threshold_per_100w: {}", suggested_fail);
    println!();

    if !whitelist_candidates.is_empty() {
        println!("{}", style("Phrases to Consider Whitelisting:").bold());
        for (phrase, count) in whitelist_candidates.iter().take(20) {
            println!("  - {} (appeared {} times)", style(phrase).cyan(), count);
        }
        println!();
    }

    // Write calibration file
    let calibration = serde_yaml::to_string(&CalibrationOutput {
        description: "Auto-generated calibration from good writing samples".into(),
        source_files: files.len(),
        total_words,
        statistics: CalibrationStats {
            avg_density_per_100w: avg_density,
            max_density_per_100w: max_density,
            avg_sentence_length: avg_sentence_length as f32,
        },
        suggested_scores: SuggestedScores {
            warn_threshold_per_100w: suggested_warn,
            fail_threshold_per_100w: suggested_fail,
        },
        whitelist_candidates: whitelist_candidates
            .into_iter()
            .take(50)
            .map(|(p, c)| WhitelistCandidate {
                phrase: p,
                count: c,
            })
            .collect(),
        category_distribution: category_counts,
    })?;

    fs::write(&args.output, calibration)?;
    println!(
        "{}",
        style(format!("Wrote calibration to: {}", args.output.display())).green()
    );

    Ok(())
}

#[derive(Debug, Serialize)]
struct CalibrationOutput {
    description: String,
    source_files: usize,
    total_words: usize,
    statistics: CalibrationStats,
    suggested_scores: SuggestedScores,
    whitelist_candidates: Vec<WhitelistCandidate>,
    category_distribution: BTreeMap<Category, usize>,
}

#[derive(Debug, Serialize)]
struct CalibrationStats {
    avg_density_per_100w: f32,
    max_density_per_100w: f32,
    avg_sentence_length: f32,
}

#[derive(Debug, Serialize)]
struct SuggestedScores {
    warn_threshold_per_100w: u32,
    fail_threshold_per_100w: u32,
}

#[derive(Debug, Serialize)]
struct WhitelistCandidate {
    phrase: String,
    count: usize,
}

fn run_flow(args: FlowArgs) -> anyhow::Result<()> {
    match args.command {
        FlowCommand::Check(check_args) => run_flow_check(check_args),
        FlowCommand::Audit(audit_args) => run_flow_audit(audit_args),
        FlowCommand::Propose(propose_args) => run_flow_propose(propose_args),
        FlowCommand::New(new_args) => run_flow_new(new_args),
        FlowCommand::Blueprint(blueprint_args) => run_flow_blueprint(blueprint_args),
        FlowCommand::Index(index_args) => run_flow_index(index_args),
        FlowCommand::Callgraph(callgraph_args) => run_flow_callgraph(callgraph_args),
        FlowCommand::Graph(graph_args) => run_flow_graph(graph_args),
    }
}

fn run_flow_check(args: FlowCheckArgs) -> anyhow::Result<()> {
    let (cfg, config_root) = load_config(&args.config)?;
    let flows_dir = resolve_flows_dir(&config_root, &args.flows);
    let report = flow_check_report(&cfg, &flows_dir)?;

    if args.json {
        let payload = serde_json::to_string_pretty(&report)?;
        println!("{payload}");
    } else {
        print_flow_check_report(&report);
    }

    if let Some(out) = &args.out {
        write_json(out, &report)?;
    }

    if report.error_count > 0 {
        std::process::exit(2);
    }

    Ok(())
}

fn run_flow_audit(args: FlowAuditArgs) -> anyhow::Result<()> {
    let (cfg, config_root) = load_config(&args.config)?;
    let flows_dir = resolve_flows_dir(&config_root, &args.flows);
    let flow_check = if args.no_flow_checks {
        None
    } else {
        Some(flow_check_report(&cfg, &flows_dir)?)
    };

    let mut ignore_globs = cfg.repo_rules.ignore_globs.clone();
    ignore_globs.extend(cfg.flow_rules.ignore_globs.clone());

    let mut audit_config = FlowAuditConfig::default();
    audit_config.ignore_globs = ignore_globs;
    audit_config.base_dir = Some(config_root.clone());
    audit_config.duplication_min_instances = cfg.flow_rules.duplication_min_instances;
    audit_config.duplication_min_tokens = cfg.flow_rules.duplication_min_tokens;
    audit_config.duplication_max_groups = cfg.flow_rules.duplication_max_groups;
    if !args.language.is_empty() {
        audit_config.languages = parse_languages(&args.language)?;
    }

    let scan_paths = if args.paths.is_empty() {
        vec![config_root.clone()]
    } else {
        args.paths.clone()
    };

    let audit = dwg_core::arch::audit_paths(&scan_paths, &audit_config)?;
    let output = FlowAuditOutput { flow_check, audit };

    if args.json {
        let payload = serde_json::to_string_pretty(&output)?;
        println!("{payload}");
    } else {
        print_flow_audit_report(&output);
    }

    if let Some(out) = &args.out {
        write_json(out, &output)?;
    }

    let flow_errors = output
        .flow_check
        .as_ref()
        .map(|r| r.error_count)
        .unwrap_or(0);
    let audit_errors = output
        .audit
        .findings
        .iter()
        .filter(|f| matches!(f.severity, dwg_core::arch::FindingSeverity::Error))
        .count();

    if flow_errors > 0 || audit_errors > 0 {
        std::process::exit(2);
    }

    Ok(())
}

fn run_flow_propose(args: FlowProposeArgs) -> anyhow::Result<()> {
    let (cfg, config_root) = load_config(&args.config)?;
    let flows_dir = resolve_flows_dir(&config_root, &args.flows);
    let flow_check = if args.no_flow_checks {
        None
    } else {
        Some(flow_check_report(&cfg, &flows_dir)?)
    };

    let mut ignore_globs = cfg.repo_rules.ignore_globs.clone();
    ignore_globs.extend(cfg.flow_rules.ignore_globs.clone());

    let mut audit_config = FlowAuditConfig::default();
    audit_config.ignore_globs = ignore_globs;
    audit_config.base_dir = Some(config_root.clone());
    audit_config.duplication_min_instances = cfg.flow_rules.duplication_min_instances;
    audit_config.duplication_min_tokens = cfg.flow_rules.duplication_min_tokens;
    audit_config.duplication_max_groups = cfg.flow_rules.duplication_max_groups;
    if !args.language.is_empty() {
        audit_config.languages = parse_languages(&args.language)?;
    }

    let scan_paths = if args.paths.is_empty() {
        vec![config_root.clone()]
    } else {
        args.paths.clone()
    };
    let audit = dwg_core::arch::audit_paths(&scan_paths, &audit_config)?;

    let markdown = render_flow_proposal(&flow_check, &audit);
    if let Some(out) = &args.out {
        write_text(out, &markdown)?;
    } else {
        print!("{markdown}");
    }

    let flow_errors = flow_check.as_ref().map(|r| r.error_count).unwrap_or(0);
    let audit_errors = audit
        .findings
        .iter()
        .filter(|f| matches!(f.severity, dwg_core::arch::FindingSeverity::Error))
        .count();
    if flow_errors > 0 || audit_errors > 0 {
        std::process::exit(2);
    }

    Ok(())
}

fn run_flow_new(args: FlowNewArgs) -> anyhow::Result<()> {
    let (cfg, config_root) = load_config(&args.config)?;
    let flows_dir = resolve_flows_dir(&config_root, &args.flows);
    fs::create_dir_all(&flows_dir)?;

    let default_filename = format!("{}.md", slugify_kebab(&args.name));
    let out_path = if let Some(out) = &args.out {
        if out.is_absolute() {
            out.clone()
        } else {
            config_root.join(out)
        }
    } else {
        flows_dir.join(default_filename)
    };

    if out_path.exists() && !args.force {
        return Err(anyhow!(
            "Refusing to overwrite {}; pass --force to overwrite",
            out_path.display()
        ));
    }

    let indirection_budget = cfg.flow_rules.indirection_budget;
    let content = render_flow_template(
        &args.name,
        &args.entrypoint,
        args.language.as_deref(),
        indirection_budget,
    );
    write_text(&out_path, &content)?;

    let rel_path = pathdiff::diff_paths(&out_path, &config_root).unwrap_or(out_path.clone());
    println!(
        "Wrote flow spec: {}",
        rel_path.to_string_lossy().replace('\\', "/")
    );
    Ok(())
}

fn run_flow_blueprint(args: FlowBlueprintArgs) -> anyhow::Result<()> {
    if let Some(cmd) = args.command {
        match cmd {
            FlowBlueprintCommand::Diff(diff) => return run_flow_blueprint_diff(diff),
        }
    }

    let (cfg, config_root) = load_config(&args.config)?;

    let mut ignore_globs = cfg.repo_rules.ignore_globs.clone();
    ignore_globs.extend(cfg.flow_rules.ignore_globs.clone());

    let scan_paths = if args.paths.is_empty() {
        vec![config_root.clone()]
    } else {
        args.paths.clone()
    };

    let blueprint_config = BlueprintConfig {
        ignore_globs,
        base_dir: Some(config_root),
    };

    let report = blueprint_paths(&scan_paths, &blueprint_config)?;

    match args.format.as_str() {
        "json" => {
            if let Some(out) = &args.out {
                write_json(out, &report)?;
            } else {
                println!("{}", serde_json::to_string_pretty(&report)?);
            }
        }
        "jsonl" => {
            let lines = blueprint_to_jsonl(&report)?;
            if let Some(out) = &args.out {
                write_text(out, &lines)?;
            } else {
                print!("{lines}");
            }
        }
        other => {
            return Err(anyhow!(
                "Unsupported format: {other} (expected json or jsonl)"
            ));
        }
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
struct BlueprintResolvedEdgeKey {
    from: String,
    to: String,
    kind: dwg_core::blueprint::EdgeKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BlueprintRenameCandidate {
    path: String,
    score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BlueprintMappingEntry {
    old: String,
    action: String,
    new: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    candidates: Vec<BlueprintRenameCandidate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BlueprintMappingFile {
    version: u32,
    before: String,
    after: String,
    mappings: Vec<BlueprintMappingEntry>,
}

#[derive(Debug, Serialize)]
struct BlueprintDiffReport {
    before: BlueprintSnapshotSummary,
    after: BlueprintSnapshotSummary,
    nodes_added: Vec<String>,
    nodes_removed: Vec<String>,
    resolved_edges_added: Vec<BlueprintResolvedEdgeKey>,
    resolved_edges_removed: Vec<BlueprintResolvedEdgeKey>,
    rename_candidates: Vec<BlueprintRenameGroup>,
    mapping_template: BlueprintMappingFile,
    mapping_check: Option<BlueprintMappingCheck>,
}

#[derive(Debug, Serialize)]
struct BlueprintSnapshotSummary {
    nodes: usize,
    edges: usize,
    edges_resolved: usize,
    errors: usize,
}

#[derive(Debug, Serialize)]
struct BlueprintRenameGroup {
    old: String,
    candidates: Vec<BlueprintRenameCandidate>,
}

#[derive(Debug, Serialize)]
struct BlueprintMappingCheck {
    unmapped: Vec<String>,
    invalid: Vec<String>,
}

fn read_blueprint_snapshot(path: &Path) -> anyhow::Result<BlueprintReport> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("Failed to read blueprint snapshot {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse blueprint snapshot {}", path.display()))
}

fn resolved_edge_keys(report: &BlueprintReport) -> BTreeSet<BlueprintResolvedEdgeKey> {
    report
        .edges
        .iter()
        .filter(|e| e.resolved)
        .filter_map(|e| {
            e.to.as_ref()
                .map(|to| (e.from.clone(), to.clone(), e.kind.clone()))
        })
        .map(|(from, to, kind)| BlueprintResolvedEdgeKey { from, to, kind })
        .collect()
}

fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count() as f32;
    let union = a.union(b).count() as f32;
    if union <= 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn build_neighbors(
    edges: &BTreeSet<BlueprintResolvedEdgeKey>,
) -> (
    BTreeMap<String, BTreeSet<String>>,
    BTreeMap<String, BTreeSet<String>>,
) {
    let mut outgoing: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut incoming: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for edge in edges {
        outgoing
            .entry(edge.from.clone())
            .or_default()
            .insert(edge.to.clone());
        incoming
            .entry(edge.to.clone())
            .or_default()
            .insert(edge.from.clone());
    }
    (outgoing, incoming)
}

fn build_blueprint_mapping_template(
    before_path: &Path,
    after_path: &Path,
    removed: &[String],
    candidates: &BTreeMap<String, Vec<BlueprintRenameCandidate>>,
) -> BlueprintMappingFile {
    let mut mappings = Vec::new();
    for old in removed {
        mappings.push(BlueprintMappingEntry {
            old: old.clone(),
            action: "unmapped".to_string(),
            new: Vec::new(),
            candidates: candidates.get(old).cloned().unwrap_or_default(),
            reason: None,
            notes: None,
        });
    }
    BlueprintMappingFile {
        version: 1,
        before: before_path.to_string_lossy().to_string(),
        after: after_path.to_string_lossy().to_string(),
        mappings,
    }
}

fn validate_blueprint_mapping(
    mapping_path: &Path,
    removed: &[String],
    after_nodes: &BTreeSet<String>,
) -> anyhow::Result<BlueprintMappingCheck> {
    let raw = fs::read_to_string(mapping_path)
        .with_context(|| format!("Failed to read mapping file {}", mapping_path.display()))?;
    let mapping: BlueprintMappingFile = serde_yaml::from_str(&raw)
        .with_context(|| format!("Failed to parse mapping file {}", mapping_path.display()))?;

    let mut by_old: BTreeMap<String, BlueprintMappingEntry> = BTreeMap::new();
    for entry in mapping.mappings {
        by_old.insert(entry.old.clone(), entry);
    }

    let mut unmapped = Vec::new();
    let mut invalid = Vec::new();

    for old in removed {
        let Some(entry) = by_old.get(old) else {
            unmapped.push(old.clone());
            continue;
        };

        let action = entry.action.trim().to_lowercase();
        if action.is_empty() || action == "unmapped" {
            unmapped.push(old.clone());
            continue;
        }

        match action.as_str() {
            "deleted" => {
                if !entry.new.is_empty() {
                    invalid.push(format!("{old}: action=deleted must not include new paths"));
                }
            }
            "moved" | "renamed" | "merged" | "split" => {
                if entry.new.is_empty() {
                    invalid.push(format!(
                        "{old}: action={action} requires at least one new path"
                    ));
                    continue;
                }
                for new_path in &entry.new {
                    if !after_nodes.contains(new_path) {
                        invalid.push(format!("{old}: maps to missing after node {new_path}"));
                    }
                }
            }
            other => {
                unmapped.push(format!("{old} (unknown action: {other})"));
            }
        }
    }

    Ok(BlueprintMappingCheck { unmapped, invalid })
}

fn run_flow_blueprint_diff(args: FlowBlueprintDiffArgs) -> anyhow::Result<()> {
    let before = read_blueprint_snapshot(&args.before)?;
    let after = read_blueprint_snapshot(&args.after)?;

    let before_nodes: BTreeSet<String> = before.nodes.iter().map(|n| n.path.clone()).collect();
    let after_nodes: BTreeSet<String> = after.nodes.iter().map(|n| n.path.clone()).collect();

    let nodes_added: Vec<String> = after_nodes.difference(&before_nodes).cloned().collect();
    let nodes_removed: Vec<String> = before_nodes.difference(&after_nodes).cloned().collect();

    let before_edges = resolved_edge_keys(&before);
    let after_edges = resolved_edge_keys(&after);

    let resolved_edges_added: Vec<BlueprintResolvedEdgeKey> =
        after_edges.difference(&before_edges).cloned().collect();
    let resolved_edges_removed: Vec<BlueprintResolvedEdgeKey> =
        before_edges.difference(&after_edges).cloned().collect();

    let (before_out, before_in) = build_neighbors(&before_edges);
    let (after_out, after_in) = build_neighbors(&after_edges);

    let mut candidate_map: BTreeMap<String, Vec<BlueprintRenameCandidate>> = BTreeMap::new();
    for old in &nodes_removed {
        let out_old = before_out.get(old).cloned().unwrap_or_default();
        let in_old = before_in.get(old).cloned().unwrap_or_default();
        let base_old = old.rsplit('/').next().unwrap_or(old.as_str());

        let mut scored: Vec<BlueprintRenameCandidate> = Vec::new();
        for new_path in &nodes_added {
            let out_new = after_out.get(new_path).cloned().unwrap_or_default();
            let in_new = after_in.get(new_path).cloned().unwrap_or_default();
            let base_new = new_path.rsplit('/').next().unwrap_or(new_path.as_str());

            let mut score = 0.65 * jaccard(&out_old, &out_new) + 0.35 * jaccard(&in_old, &in_new);
            if base_old == base_new {
                score += 0.2;
            }
            if score > 1.0 {
                score = 1.0;
            }
            if score <= 0.0 {
                continue;
            }
            scored.push(BlueprintRenameCandidate {
                path: new_path.clone(),
                score: (score * 1000.0).round() / 1000.0,
            });
        }

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(args.max_candidates.max(1));

        candidate_map.insert(old.clone(), scored);
    }

    let rename_candidates: Vec<BlueprintRenameGroup> = nodes_removed
        .iter()
        .filter_map(|old| {
            let candidates = candidate_map.get(old).cloned().unwrap_or_default();
            if candidates.is_empty() {
                return None;
            }
            Some(BlueprintRenameGroup {
                old: old.clone(),
                candidates,
            })
        })
        .collect();

    let mapping_template =
        build_blueprint_mapping_template(&args.before, &args.after, &nodes_removed, &candidate_map);

    if let Some(path) = &args.write_mapping {
        let yaml = serde_yaml::to_string(&mapping_template)?;
        write_text(path, &yaml)?;
    }

    let mapping_check = if let Some(path) = &args.require_mapping {
        Some(validate_blueprint_mapping(
            path,
            &nodes_removed,
            &after_nodes,
        )?)
    } else {
        None
    };

    let report = BlueprintDiffReport {
        before: BlueprintSnapshotSummary {
            nodes: before.stats.nodes,
            edges: before.stats.edges,
            edges_resolved: before.stats.edges_resolved,
            errors: before.errors.len(),
        },
        after: BlueprintSnapshotSummary {
            nodes: after.stats.nodes,
            edges: after.stats.edges,
            edges_resolved: after.stats.edges_resolved,
            errors: after.errors.len(),
        },
        nodes_added,
        nodes_removed,
        resolved_edges_added,
        resolved_edges_removed,
        rename_candidates,
        mapping_template,
        mapping_check,
    };

    if args.md {
        let md = render_blueprint_diff_markdown(&args.before, &args.after, &report)?;
        if let Some(out) = &args.out {
            write_text(out, &md)?;
        } else {
            print!("{md}");
        }
    } else if let Some(out) = &args.out {
        write_json(out, &report)?;
    } else {
        println!("{}", serde_json::to_string_pretty(&report)?);
    }

    if let Some(check) = &report.mapping_check {
        if !check.unmapped.is_empty() || !check.invalid.is_empty() {
            std::process::exit(2);
        }
    }

    Ok(())
}

fn render_blueprint_diff_markdown(
    before_path: &Path,
    after_path: &Path,
    report: &BlueprintDiffReport,
) -> anyhow::Result<String> {
    let mut out = String::new();
    out.push_str("# Blueprint diff\n\n");
    out.push_str(&format!(
        "- Before: `{}` ({} files, {} resolved edges)\n",
        before_path.to_string_lossy().replace('\\', "/"),
        report.before.nodes,
        report.before.edges_resolved
    ));
    out.push_str(&format!(
        "- After: `{}` ({} files, {} resolved edges)\n\n",
        after_path.to_string_lossy().replace('\\', "/"),
        report.after.nodes,
        report.after.edges_resolved
    ));

    out.push_str("## Nodes\n");
    out.push_str(&format!("- Added: {}\n", report.nodes_added.len()));
    out.push_str(&format!("- Removed: {}\n\n", report.nodes_removed.len()));
    if !report.nodes_added.is_empty() {
        out.push_str("### Added\n");
        for path in &report.nodes_added {
            out.push_str(&format!("- `{path}`\n"));
        }
        out.push('\n');
    }
    if !report.nodes_removed.is_empty() {
        out.push_str("### Removed\n");
        for path in &report.nodes_removed {
            out.push_str(&format!("- `{path}`\n"));
        }
        out.push('\n');
    }

    out.push_str("## Resolved edges\n");
    out.push_str(&format!("- Added: {}\n", report.resolved_edges_added.len()));
    out.push_str(&format!(
        "- Removed: {}\n\n",
        report.resolved_edges_removed.len()
    ));
    if !report.resolved_edges_added.is_empty() {
        out.push_str("### Added\n");
        for edge in &report.resolved_edges_added {
            out.push_str(&format!(
                "- `{}` → `{}` ({:?})\n",
                edge.from, edge.to, edge.kind
            ));
        }
        out.push('\n');
    }
    if !report.resolved_edges_removed.is_empty() {
        out.push_str("### Removed\n");
        for edge in &report.resolved_edges_removed {
            out.push_str(&format!(
                "- `{}` → `{}` ({:?})\n",
                edge.from, edge.to, edge.kind
            ));
        }
        out.push('\n');
    }

    if !report.rename_candidates.is_empty() {
        out.push_str("## Rename candidates\n");
        for group in &report.rename_candidates {
            out.push_str(&format!("- `{}`\n", group.old));
            for cand in &group.candidates {
                out.push_str(&format!("  - `{}` (score {})\n", cand.path, cand.score));
            }
        }
        out.push('\n');
    }

    if let Some(check) = &report.mapping_check {
        out.push_str("## Mapping check\n");
        out.push_str(&format!("- Unmapped: {}\n", check.unmapped.len()));
        out.push_str(&format!("- Invalid: {}\n\n", check.invalid.len()));
        if !check.unmapped.is_empty() {
            out.push_str("### Unmapped\n");
            for item in &check.unmapped {
                out.push_str(&format!("- `{item}`\n"));
            }
            out.push('\n');
        }
        if !check.invalid.is_empty() {
            out.push_str("### Invalid\n");
            for item in &check.invalid {
                out.push_str(&format!("- {item}\n"));
            }
            out.push('\n');
        }
    }

    out.push_str("## Mapping template\n\n");
    out.push_str("```yaml\n");
    out.push_str(&serde_yaml::to_string(&report.mapping_template)?);
    out.push_str("```\n");
    Ok(out)
}

fn run_flow_graph(args: FlowGraphArgs) -> anyhow::Result<()> {
    use dwg_core::arch::analyze_rust_logic;
    use dwg_core::cfg::{build_cfg_rust, CfgLanguage, ControlFlowGraph};

    let path = &args.file;
    if !path.exists() {
        return Err(anyhow!("File not found: {}", path.display()));
    }

    let text = fs::read_to_string(path)?;
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");

    let cfgs: Vec<ControlFlowGraph> = match ext {
        "rs" => {
            let file =
                syn::parse_file(&text).map_err(|e| anyhow!("Failed to parse Rust file: {}", e))?;

            let mut result = Vec::new();
            for item in &file.items {
                match item {
                    syn::Item::Fn(item_fn) => {
                        let fn_name = item_fn.sig.ident.to_string();
                        if let Some(ref target) = args.r#fn {
                            if &fn_name != target {
                                continue;
                            }
                        }
                        result.push(build_cfg_rust(item_fn, path));
                    }
                    syn::Item::Impl(impl_block) => {
                        let self_ty = match &*impl_block.self_ty {
                            syn::Type::Path(tp) => tp
                                .path
                                .segments
                                .last()
                                .map(|s| s.ident.to_string())
                                .unwrap_or_else(|| "Unknown".into()),
                            _ => "Unknown".into(),
                        };
                        for impl_item in &impl_block.items {
                            if let syn::ImplItem::Fn(method) = impl_item {
                                let fn_name = method.sig.ident.to_string();
                                let full_name = format!("{}::{}", self_ty, fn_name);
                                if let Some(ref target) = args.r#fn {
                                    if &fn_name != target && &full_name != target {
                                        continue;
                                    }
                                }
                                result.push(dwg_core::cfg::build_cfg_rust_method(
                                    method, path, &self_ty,
                                ));
                            }
                        }
                    }
                    _ => {}
                }
            }
            result
        }
        "ts" | "tsx" | "js" | "jsx" => {
            // For TypeScript/JavaScript, use tree-sitter
            let is_ts = ext == "ts" || ext == "tsx";
            let mut parser = tree_sitter::Parser::new();
            let language = if is_ts {
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
            } else {
                tree_sitter_javascript::LANGUAGE.into()
            };
            parser.set_language(&language)?;

            let tree = parser
                .parse(&text, None)
                .ok_or_else(|| anyhow!("Failed to parse TypeScript/JavaScript file"))?;

            let mut result = Vec::new();
            let source = text.as_bytes();

            fn visit_node(
                node: tree_sitter::Node,
                source: &[u8],
                path: &Path,
                is_ts: bool,
                target_fn: &Option<String>,
                result: &mut Vec<ControlFlowGraph>,
            ) {
                match node.kind() {
                    "function_declaration"
                    | "function"
                    | "arrow_function"
                    | "method_definition" => {
                        // Check function name if target is specified
                        if let Some(target) = target_fn {
                            if let Some(name_node) = node.child_by_field_name("name") {
                                if let Ok(name) = name_node.utf8_text(source) {
                                    if name != target {
                                        return;
                                    }
                                }
                            }
                        }
                        if let Some(cfg) = dwg_core::cfg::build_cfg_ts(node, source, path, is_ts) {
                            result.push(cfg);
                        }
                    }
                    _ => {
                        let mut cursor = node.walk();
                        for child in node.children(&mut cursor) {
                            visit_node(child, source, path, is_ts, target_fn, result);
                        }
                    }
                }
            }

            visit_node(
                tree.root_node(),
                source,
                path,
                is_ts,
                &args.r#fn,
                &mut result,
            );
            result
        }
        "py" => {
            // For Python, use tree-sitter
            let mut parser = tree_sitter::Parser::new();
            let language = tree_sitter_python::LANGUAGE.into();
            parser.set_language(&language)?;

            let tree = parser
                .parse(&text, None)
                .ok_or_else(|| anyhow!("Failed to parse Python file"))?;

            let mut result = Vec::new();
            let source = text.as_bytes();

            fn visit_node(
                node: tree_sitter::Node,
                source: &[u8],
                path: &Path,
                target_fn: &Option<String>,
                result: &mut Vec<ControlFlowGraph>,
            ) {
                if node.kind() == "function_definition" {
                    if let Some(target) = target_fn {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(source) {
                                if name != target {
                                    return;
                                }
                            }
                        }
                    }
                    if let Some(cfg) = dwg_core::cfg::build_cfg_python(node, source, path) {
                        result.push(cfg);
                    }
                } else {
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        visit_node(child, source, path, target_fn, result);
                    }
                }
            }

            visit_node(tree.root_node(), source, path, &args.r#fn, &mut result);
            result
        }
        _ => {
            return Err(anyhow!("Unsupported file type: {}", ext));
        }
    };

    if cfgs.is_empty() {
        if let Some(ref target) = args.r#fn {
            return Err(anyhow!(
                "Function '{}' not found in {}",
                target,
                path.display()
            ));
        } else {
            return Err(anyhow!("No functions found in {}", path.display()));
        }
    }

    // Include logic findings if requested
    let logic_findings = if args.with_logic && ext == "rs" {
        let result = analyze_rust_logic(path, &text);
        let mut all: Vec<dwg_core::arch::FlowFinding> = Vec::new();
        all.extend(result.exit_path_findings);
        all.extend(result.dead_branch_findings);
        all.extend(result.validation_gap_findings);
        all.extend(result.error_escalation_findings);
        Some(all)
    } else {
        None
    };

    // Output
    #[derive(Serialize)]
    struct GraphOutput {
        cfgs: Vec<CfgOutputItem>,
        #[serde(skip_serializing_if = "Option::is_none")]
        logic_findings: Option<Vec<dwg_core::arch::FlowFinding>>,
    }

    #[derive(Serialize)]
    struct CfgOutputItem {
        name: String,
        file: String,
        start_line: u32,
        language: CfgLanguage,
        nodes: usize,
        edges: usize,
        exits: Vec<u32>,
        unreachable: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        mermaid: Option<String>,
    }

    let format = args.format.to_lowercase();
    let items: Vec<CfgOutputItem> = cfgs
        .iter()
        .map(|cfg| CfgOutputItem {
            name: cfg.name.clone(),
            file: cfg.path.to_string_lossy().replace('\\', "/"),
            start_line: cfg.start_line,
            language: cfg.language,
            nodes: cfg.nodes.len(),
            edges: cfg.edges.len(),
            exits: cfg.exits.clone(),
            unreachable: cfg.unreachable_nodes().len(),
            mermaid: if args.include_mermaid {
                Some(cfg.to_mermaid())
            } else {
                None
            },
        })
        .collect();

    let output = GraphOutput {
        cfgs: items,
        logic_findings,
    };

    let output_str = if format == "mermaid" {
        // For mermaid, just output the diagram(s)
        cfgs.iter()
            .map(|cfg| format!("## {}\n\n```mermaid\n{}\n```\n", cfg.name, cfg.to_mermaid()))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        serde_json::to_string_pretty(&output)?
    };

    if let Some(out) = &args.out {
        fs::write(out, &output_str)?;
        println!("Wrote CFG output to: {}", out.display());
    } else {
        println!("{}", output_str);
    }

    Ok(())
}

fn run_flow_index(args: FlowIndexArgs) -> anyhow::Result<()> {
    use dwg_core::cfg::CfgLanguage;

    #[derive(Debug, Serialize)]
    struct FlowIndexItem {
        display_name: String,
        target_name: String,
        file: String,
        file_display: String,
        start_line: u32,
        language: CfgLanguage,
        kind: String,
    }

    #[derive(Debug, Serialize)]
    struct FlowIndexOutput {
        files_scanned: usize,
        functions: usize,
        by_language: BTreeMap<String, usize>,
        items: Vec<FlowIndexItem>,
    }

    let (cfg, config_root) = load_config(&args.config)?;
    let mut ignore_globs = cfg.repo_rules.ignore_globs.clone();
    ignore_globs.extend(cfg.flow_rules.ignore_globs.clone());
    let ignore_set = build_ignore_set(&ignore_globs)?;

    let files = collect_code_files(&args.paths, ignore_set.as_ref())?;
    let mut by_language: BTreeMap<String, usize> = BTreeMap::new();
    let mut items: Vec<FlowIndexItem> = Vec::new();

    for path in &files {
        let rel = pathdiff::diff_paths(path, &config_root).unwrap_or_else(|| path.clone());
        let file_display = rel.to_string_lossy().replace('\\', "/");
        let file_abs = path.to_string_lossy().replace('\\', "/");
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");

        match ext {
            "rs" => {
                let before = items.len();
                let text = fs::read_to_string(path)?;
                let file = syn::parse_file(&text)
                    .map_err(|e| anyhow!("Failed to parse Rust file: {}", e))?;

                for item in &file.items {
                    match item {
                        syn::Item::Fn(item_fn) => {
                            let name = item_fn.sig.ident.to_string();
                            let start_line = item_fn.sig.ident.span().start().line as u32;
                            items.push(FlowIndexItem {
                                display_name: name.clone(),
                                target_name: name,
                                file: file_abs.clone(),
                                file_display: file_display.clone(),
                                start_line,
                                language: CfgLanguage::Rust,
                                kind: "function".to_string(),
                            });
                        }
                        syn::Item::Impl(impl_block) => {
                            let self_ty = match &*impl_block.self_ty {
                                syn::Type::Path(tp) => tp
                                    .path
                                    .segments
                                    .last()
                                    .map(|s| s.ident.to_string())
                                    .unwrap_or_else(|| "Unknown".into()),
                                _ => "Unknown".into(),
                            };
                            for impl_item in &impl_block.items {
                                if let syn::ImplItem::Fn(method) = impl_item {
                                    let fn_name = method.sig.ident.to_string();
                                    let display_name = format!("{}::{}", self_ty, fn_name);
                                    let start_line = method.sig.ident.span().start().line as u32;
                                    items.push(FlowIndexItem {
                                        display_name,
                                        target_name: format!("{}::{}", self_ty, fn_name),
                                        file: file_abs.clone(),
                                        file_display: file_display.clone(),
                                        start_line,
                                        language: CfgLanguage::Rust,
                                        kind: "method".to_string(),
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }
                let added = items.len().saturating_sub(before);
                if added > 0 {
                    *by_language.entry("rust".to_string()).or_insert(0) += added;
                }
            }
            "ts" | "tsx" | "js" | "jsx" => {
                let before = items.len();
                let text = fs::read_to_string(path)?;
                let source = text.as_bytes();
                let is_ts = ext == "ts" || ext == "tsx";
                let mut parser = tree_sitter::Parser::new();
                let language = if is_ts {
                    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
                } else {
                    tree_sitter_javascript::LANGUAGE.into()
                };
                parser.set_language(&language)?;

                let tree = parser
                    .parse(&text, None)
                    .ok_or_else(|| anyhow!("Failed to parse TypeScript/JavaScript file"))?;

                fn node_text(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
                    node.utf8_text(source).ok().map(|s| s.to_string())
                }

                fn class_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
                    node.child_by_field_name("name")
                        .and_then(|n| node_text(n, source))
                }

                fn visit_node(
                    node: tree_sitter::Node,
                    source: &[u8],
                    file_abs: &str,
                    file_display: &str,
                    language: CfgLanguage,
                    class_stack: &mut Vec<String>,
                    items: &mut Vec<FlowIndexItem>,
                ) {
                    match node.kind() {
                        "class_declaration" | "class" => {
                            if let Some(name) = class_name(node, source) {
                                class_stack.push(name);
                                let mut cursor = node.walk();
                                for child in node.children(&mut cursor) {
                                    visit_node(
                                        child,
                                        source,
                                        file_abs,
                                        file_display,
                                        language,
                                        class_stack,
                                        items,
                                    );
                                }
                                class_stack.pop();
                            } else {
                                let mut cursor = node.walk();
                                for child in node.children(&mut cursor) {
                                    visit_node(
                                        child,
                                        source,
                                        file_abs,
                                        file_display,
                                        language,
                                        class_stack,
                                        items,
                                    );
                                }
                            }
                        }
                        "function_declaration" => {
                            let name = node
                                .child_by_field_name("name")
                                .and_then(|n| node_text(n, source));
                            if let Some(name) = name {
                                let start_line = (node.start_position().row + 1) as u32;
                                items.push(FlowIndexItem {
                                    display_name: name.clone(),
                                    target_name: name,
                                    file: file_abs.to_string(),
                                    file_display: file_display.to_string(),
                                    start_line,
                                    language,
                                    kind: "function".to_string(),
                                });
                            }
                        }
                        "method_definition" => {
                            let name = node
                                .child_by_field_name("name")
                                .and_then(|n| node_text(n, source));
                            if let Some(name) = name {
                                let start_line = (node.start_position().row + 1) as u32;
                                let display_name = class_stack
                                    .last()
                                    .map(|c| format!("{}.{}", c, name))
                                    .unwrap_or_else(|| name.clone());
                                items.push(FlowIndexItem {
                                    display_name,
                                    target_name: name,
                                    file: file_abs.to_string(),
                                    file_display: file_display.to_string(),
                                    start_line,
                                    language,
                                    kind: "method".to_string(),
                                });
                            }
                        }
                        "variable_declarator" => {
                            let name_node = node.child_by_field_name("name");
                            let value_node = node.child_by_field_name("value");
                            if let (Some(name_node), Some(value_node)) = (name_node, value_node) {
                                let is_fn = matches!(
                                    value_node.kind(),
                                    "function" | "arrow_function" | "generator_function"
                                );
                                if is_fn {
                                    if let Some(name) = node_text(name_node, source) {
                                        let start_line =
                                            (value_node.start_position().row + 1) as u32;
                                        items.push(FlowIndexItem {
                                            display_name: name.clone(),
                                            target_name: name,
                                            file: file_abs.to_string(),
                                            file_display: file_display.to_string(),
                                            start_line,
                                            language,
                                            kind: "function".to_string(),
                                        });
                                    }
                                }
                            }
                        }
                        _ => {
                            let mut cursor = node.walk();
                            for child in node.children(&mut cursor) {
                                visit_node(
                                    child,
                                    source,
                                    file_abs,
                                    file_display,
                                    language,
                                    class_stack,
                                    items,
                                );
                            }
                        }
                    }
                }

                let lang = if is_ts {
                    CfgLanguage::TypeScript
                } else {
                    CfgLanguage::JavaScript
                };
                let mut class_stack: Vec<String> = Vec::new();
                visit_node(
                    tree.root_node(),
                    source,
                    &file_abs,
                    &file_display,
                    lang,
                    &mut class_stack,
                    &mut items,
                );
                let key = if is_ts { "typescript" } else { "javascript" };
                let added = items.len().saturating_sub(before);
                if added > 0 {
                    *by_language.entry(key.to_string()).or_insert(0) += added;
                }
            }
            "py" => {
                let before = items.len();
                let text = fs::read_to_string(path)?;
                let source = text.as_bytes();
                let mut parser = tree_sitter::Parser::new();
                let language = tree_sitter_python::LANGUAGE.into();
                parser.set_language(&language)?;

                let tree = parser
                    .parse(&text, None)
                    .ok_or_else(|| anyhow!("Failed to parse Python file"))?;

                fn node_text(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
                    node.utf8_text(source).ok().map(|s| s.to_string())
                }

                fn visit_node(
                    node: tree_sitter::Node,
                    source: &[u8],
                    file_abs: &str,
                    file_display: &str,
                    class_stack: &mut Vec<String>,
                    items: &mut Vec<FlowIndexItem>,
                ) {
                    match node.kind() {
                        "class_definition" => {
                            if let Some(name_node) = node.child_by_field_name("name") {
                                if let Some(name) = node_text(name_node, source) {
                                    class_stack.push(name);
                                    let mut cursor = node.walk();
                                    for child in node.children(&mut cursor) {
                                        visit_node(
                                            child,
                                            source,
                                            file_abs,
                                            file_display,
                                            class_stack,
                                            items,
                                        );
                                    }
                                    class_stack.pop();
                                    return;
                                }
                            }
                            let mut cursor = node.walk();
                            for child in node.children(&mut cursor) {
                                visit_node(
                                    child,
                                    source,
                                    file_abs,
                                    file_display,
                                    class_stack,
                                    items,
                                );
                            }
                        }
                        "function_definition" => {
                            if let Some(name_node) = node.child_by_field_name("name") {
                                if let Some(name) = node_text(name_node, source) {
                                    let start_line = (node.start_position().row + 1) as u32;
                                    let display_name = class_stack
                                        .last()
                                        .map(|c| format!("{}.{}", c, name))
                                        .unwrap_or_else(|| name.clone());
                                    items.push(FlowIndexItem {
                                        display_name,
                                        target_name: name,
                                        file: file_abs.to_string(),
                                        file_display: file_display.to_string(),
                                        start_line,
                                        language: CfgLanguage::Python,
                                        kind: if class_stack.is_empty() {
                                            "function".to_string()
                                        } else {
                                            "method".to_string()
                                        },
                                    });
                                }
                            }
                        }
                        _ => {
                            let mut cursor = node.walk();
                            for child in node.children(&mut cursor) {
                                visit_node(
                                    child,
                                    source,
                                    file_abs,
                                    file_display,
                                    class_stack,
                                    items,
                                );
                            }
                        }
                    }
                }

                let mut class_stack: Vec<String> = Vec::new();
                visit_node(
                    tree.root_node(),
                    source,
                    &file_abs,
                    &file_display,
                    &mut class_stack,
                    &mut items,
                );
                let added = items.len().saturating_sub(before);
                if added > 0 {
                    *by_language.entry("python".to_string()).or_insert(0) += added;
                }
            }
            _ => {}
        }
    }

    items.sort_by(|a, b| {
        let file_cmp = a.file.cmp(&b.file);
        if file_cmp != std::cmp::Ordering::Equal {
            return file_cmp;
        }
        let line_cmp = a.start_line.cmp(&b.start_line);
        if line_cmp != std::cmp::Ordering::Equal {
            return line_cmp;
        }
        a.display_name.cmp(&b.display_name)
    });

    let output = FlowIndexOutput {
        files_scanned: files.len(),
        functions: items.len(),
        by_language,
        items,
    };

    let output_str = serde_json::to_string_pretty(&output)?;

    if let Some(out) = &args.out {
        fs::write(out, &output_str)?;
        println!("Wrote flow index to: {}", out.display());
    } else {
        println!("{output_str}");
    }

    Ok(())
}

fn run_flow_callgraph(args: FlowCallgraphArgs) -> anyhow::Result<()> {
    use std::collections::HashMap;
    use syn::spanned::Spanned;
    use syn::visit::Visit;

    #[derive(Debug, Serialize)]
    struct CallgraphNode {
        id: String,
        display_name: String,
        target_name: String,
        file: String,
        file_display: String,
        start_line: u32,
        kind: String,
    }

    #[derive(Debug, Serialize)]
    struct CallgraphEdge {
        from: String,
        to: Option<String>,
        to_raw: String,
        kind: String,
        line: Option<u32>,
        resolved: bool,
    }

    #[derive(Debug, Serialize)]
    struct CallgraphStats {
        files_scanned: usize,
        nodes: usize,
        edges: usize,
        edges_resolved: usize,
    }

    #[derive(Debug, Serialize)]
    struct CallgraphError {
        path: String,
        message: String,
    }

    #[derive(Debug, Serialize)]
    struct CallgraphOutput {
        nodes: Vec<CallgraphNode>,
        edges: Vec<CallgraphEdge>,
        stats: CallgraphStats,
        errors: Vec<CallgraphError>,
    }

    fn normalize_path_display(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    fn rust_path_text(path: &syn::Path) -> String {
        path.segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::")
    }

    fn rust_callee_text(expr: &syn::Expr) -> Option<String> {
        match expr {
            syn::Expr::Path(p) => Some(rust_path_text(&p.path)),
            syn::Expr::Paren(p) => rust_callee_text(&p.expr),
            syn::Expr::Group(g) => rust_callee_text(&g.expr),
            syn::Expr::Reference(r) => rust_callee_text(&r.expr),
            syn::Expr::Unary(u) => rust_callee_text(&u.expr),
            _ => None,
        }
    }

    fn resolve_callee(
        file_display: &str,
        raw: &str,
        by_file_target: &HashMap<(String, String), Vec<String>>,
        by_target: &HashMap<String, Vec<String>>,
    ) -> Option<String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }

        let mut candidates: Vec<String> = Vec::new();
        candidates.push(raw.to_string());

        if raw.contains("::") {
            let parts: Vec<&str> = raw.split("::").filter(|s| !s.is_empty()).collect();
            if parts.len() >= 2 {
                candidates.push(format!("{}::{}", parts[parts.len() - 2], parts[parts.len() - 1]));
            }
            if let Some(last) = parts.last() {
                candidates.push((*last).to_string());
            }
        } else {
            candidates.push(raw.to_string());
        }

        for cand in candidates {
            let key = (file_display.to_string(), cand.clone());
            if let Some(ids) = by_file_target.get(&key) {
                if ids.len() == 1 {
                    return Some(ids[0].clone());
                }
            }
            if let Some(ids) = by_target.get(&cand) {
                if ids.len() == 1 {
                    return Some(ids[0].clone());
                }
            }
        }

        None
    }

    // Load config and collect Rust files (call graph is Rust-only for now).
    let (cfg, config_root) = load_config(&args.config)?;
    let mut ignore_globs = cfg.repo_rules.ignore_globs.clone();
    ignore_globs.extend(cfg.flow_rules.ignore_globs.clone());
    let ignore_set = build_ignore_set(&ignore_globs)?;

    let code_files = collect_code_files(&args.paths, ignore_set.as_ref())?;
    let files: Vec<PathBuf> = code_files
        .into_iter()
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("rs"))
        .collect();

    // Parse files and index functions/methods.
    struct ParsedRustFile {
        file_display: String,
        file_abs: String,
        ast: syn::File,
    }

    let mut parsed: Vec<ParsedRustFile> = Vec::new();
    let mut errors: Vec<CallgraphError> = Vec::new();

    for path in &files {
        let rel = pathdiff::diff_paths(path, &config_root).unwrap_or_else(|| path.clone());
        let file_display = normalize_path_display(&rel);
        let file_abs = normalize_path_display(path);
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                errors.push(CallgraphError {
                    path: file_display.clone(),
                    message: format!("Failed to read: {e}"),
                });
                continue;
            }
        };
        match syn::parse_file(&text) {
            Ok(ast) => parsed.push(ParsedRustFile {
                file_display,
                file_abs,
                ast,
            }),
            Err(e) => {
                errors.push(CallgraphError {
                    path: file_display.clone(),
                    message: format!("Failed to parse Rust file: {e}"),
                });
            }
        }
    }

    let mut nodes: Vec<CallgraphNode> = Vec::new();
    let mut by_file_target: HashMap<(String, String), Vec<String>> = HashMap::new();
    let mut by_target: HashMap<String, Vec<String>> = HashMap::new();

    for file in &parsed {
        for item in &file.ast.items {
            match item {
                syn::Item::Fn(item_fn) => {
                    let name = item_fn.sig.ident.to_string();
                    let start_line = item_fn.sig.ident.span().start().line as u32;
                    let id = format!("{}::{}", file.file_display, name);
                    nodes.push(CallgraphNode {
                        id: id.clone(),
                        display_name: name.clone(),
                        target_name: name.clone(),
                        file: file.file_abs.clone(),
                        file_display: file.file_display.clone(),
                        start_line,
                        kind: "function".to_string(),
                    });
                    by_file_target
                        .entry((file.file_display.clone(), name.clone()))
                        .or_default()
                        .push(id.clone());
                    by_target.entry(name).or_default().push(id);
                }
                syn::Item::Impl(impl_block) => {
                    let self_ty = match &*impl_block.self_ty {
                        syn::Type::Path(tp) => tp
                            .path
                            .segments
                            .last()
                            .map(|s| s.ident.to_string())
                            .unwrap_or_else(|| "Unknown".into()),
                        _ => "Unknown".into(),
                    };
                    for impl_item in &impl_block.items {
                        if let syn::ImplItem::Fn(method) = impl_item {
                            let fn_name = method.sig.ident.to_string();
                            let display_name = format!("{self_ty}::{fn_name}");
                            let start_line = method.sig.ident.span().start().line as u32;
                            let id = format!("{}::{}", file.file_display, display_name);
                            nodes.push(CallgraphNode {
                                id: id.clone(),
                                display_name: display_name.clone(),
                                target_name: display_name.clone(),
                                file: file.file_abs.clone(),
                                file_display: file.file_display.clone(),
                                start_line,
                                kind: "method".to_string(),
                            });
                            by_file_target
                                .entry((file.file_display.clone(), display_name.clone()))
                                .or_default()
                                .push(id.clone());
                            by_target.entry(display_name).or_default().push(id);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Collect call edges.
    let mut edges: Vec<CallgraphEdge> = Vec::new();
    let mut edges_resolved = 0usize;

    struct CallVisitor {
        calls: Vec<(String, u32)>,
        max: usize,
    }

    impl<'ast> syn::visit::Visit<'ast> for CallVisitor {
        fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
            if self.calls.len() < self.max {
                if let Some(raw) = rust_callee_text(&node.func) {
                    let line = node.span().start().line as u32;
                    self.calls.push((raw, line));
                }
            }
            syn::visit::visit_expr_call(self, node);
        }

        fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
            if self.calls.len() < self.max {
                let raw = node.method.to_string();
                let line = node.span().start().line as u32;
                self.calls.push((raw, line));
            }
            syn::visit::visit_expr_method_call(self, node);
        }
    }

    for file in &parsed {
        for item in &file.ast.items {
            match item {
                syn::Item::Fn(item_fn) => {
                    let name = item_fn.sig.ident.to_string();
                    let caller_id = format!("{}::{}", file.file_display, name);
                    let mut visitor = CallVisitor {
                        calls: Vec::new(),
                        max: args.max_calls_per_fn,
                    };
                    visitor.visit_block(&item_fn.block);
                    for (raw, line) in visitor.calls {
                        let to = resolve_callee(
                            &file.file_display,
                            &raw,
                            &by_file_target,
                            &by_target,
                        );
                        let resolved = to.is_some();
                        if resolved {
                            edges_resolved += 1;
                        } else if args.resolved_only {
                            continue;
                        }
                        edges.push(CallgraphEdge {
                            from: caller_id.clone(),
                            to,
                            to_raw: raw,
                            kind: "call".to_string(),
                            line: Some(line),
                            resolved,
                        });
                    }
                }
                syn::Item::Impl(impl_block) => {
                    let self_ty = match &*impl_block.self_ty {
                        syn::Type::Path(tp) => tp
                            .path
                            .segments
                            .last()
                            .map(|s| s.ident.to_string())
                            .unwrap_or_else(|| "Unknown".into()),
                        _ => "Unknown".into(),
                    };
                    for impl_item in &impl_block.items {
                        if let syn::ImplItem::Fn(method) = impl_item {
                            let fn_name = method.sig.ident.to_string();
                            let display_name = format!("{self_ty}::{fn_name}");
                            let caller_id =
                                format!("{}::{}", file.file_display, display_name);
                            let mut visitor = CallVisitor {
                                calls: Vec::new(),
                                max: args.max_calls_per_fn,
                            };
                            visitor.visit_block(&method.block);
                            for (raw, line) in visitor.calls {
                                let to = resolve_callee(
                                    &file.file_display,
                                    &raw,
                                    &by_file_target,
                                    &by_target,
                                );
                                let resolved = to.is_some();
                                if resolved {
                                    edges_resolved += 1;
                                } else if args.resolved_only {
                                    continue;
                                }
                                edges.push(CallgraphEdge {
                                    from: caller_id.clone(),
                                    to,
                                    to_raw: raw,
                                    kind: "call".to_string(),
                                    line: Some(line),
                                    resolved,
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let node_count = nodes.len();
    let edge_count = edges.len();

    let output = CallgraphOutput {
        nodes,
        edges,
        stats: CallgraphStats {
            files_scanned: parsed.len(),
            nodes: node_count,
            edges: edge_count,
            edges_resolved,
        },
        errors,
    };

    let format = args.format.to_lowercase();
    let output_str = if format == "jsonl" {
        let mut out = String::new();
        out.push_str(&serde_json::to_string(&serde_json::json!({
            "type": "stats",
            "stats": &output.stats,
        }))?);
        out.push('\n');
        for node in &output.nodes {
            out.push_str(&serde_json::to_string(&serde_json::json!({
                "type": "node",
                "node": node,
            }))?);
            out.push('\n');
        }
        for edge in &output.edges {
            out.push_str(&serde_json::to_string(&serde_json::json!({
                "type": "edge",
                "edge": edge,
            }))?);
            out.push('\n');
        }
        for err in &output.errors {
            out.push_str(&serde_json::to_string(&serde_json::json!({
                "type": "error",
                "error": err,
            }))?);
            out.push('\n');
        }
        out
    } else {
        serde_json::to_string_pretty(&output)?
    };

    if let Some(out) = &args.out {
        fs::write(out, &output_str)?;
        println!("Wrote flow call graph to: {}", out.display());
    } else {
        println!("{output_str}");
    }

    Ok(())
}

fn resolve_flows_dir(config_root: &Path, flows: &Path) -> PathBuf {
    if flows.is_absolute() {
        flows.to_path_buf()
    } else {
        config_root.join(flows)
    }
}

fn flow_check_report(cfg: &Config, flows_dir: &Path) -> anyhow::Result<FlowCheckReport> {
    let mut ignore_globs = cfg.repo_rules.ignore_globs.clone();
    ignore_globs.extend(cfg.flow_rules.ignore_globs.clone());
    let ignore_set = build_ignore_set(&ignore_globs)?;

    let flow_files = collect_flow_files(flows_dir, ignore_set.as_ref())?;
    let mut files = Vec::new();
    let mut error_count = 0;
    let mut warning_count = 0;

    if flow_files.is_empty() {
        // No flow specs is a warning, not an error - flow specs are optional
        // The audit can still run without them to detect entropy patterns
        let issue = FlowSpecIssue {
            severity: IssueSeverity::Warning,
            field: None,
            message: format!(
                "No flow specs found in {} (flow specs are optional)",
                flows_dir.display()
            ),
        };
        warning_count += 1;
        files.push(FlowCheckFile {
            path: flows_dir.to_string_lossy().replace('\\', "/"),
            issues: vec![issue],
        });
        return Ok(FlowCheckReport {
            files,
            error_count,
            warning_count,
        });
    }

    for path in flow_files {
        let text = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        match dwg_core::flow::parse_flow_spec(&path, &text) {
            Ok(doc) => {
                let issues = doc.spec.validate(&cfg.flow_rules);
                for issue in &issues {
                    match issue.severity {
                        IssueSeverity::Error => error_count += 1,
                        IssueSeverity::Warning => warning_count += 1,
                    }
                }
                files.push(FlowCheckFile {
                    path: path.to_string_lossy().replace('\\', "/"),
                    issues,
                });
            }
            Err(err) => {
                error_count += 1;
                files.push(FlowCheckFile {
                    path: path.to_string_lossy().replace('\\', "/"),
                    issues: vec![FlowSpecIssue {
                        severity: IssueSeverity::Error,
                        field: None,
                        message: err.to_string(),
                    }],
                });
            }
        }
    }

    Ok(FlowCheckReport {
        files,
        error_count,
        warning_count,
    })
}

fn collect_flow_files(flows_dir: &Path, ignore: Option<&GlobSet>) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if flows_dir.is_dir() {
        let mut walker = WalkDir::new(flows_dir).into_iter();
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
            if is_flow_spec(path) {
                files.push(path.to_path_buf());
            }
        }
    } else if flows_dir.is_file() && is_flow_spec(flows_dir) {
        files.push(flows_dir.to_path_buf());
    }
    Ok(files)
}

fn is_flow_spec(path: &Path) -> bool {
    match path.extension().and_then(|s| s.to_str()) {
        Some(ext) => matches!(
            ext.to_lowercase().as_str(),
            "md" | "markdown" | "yml" | "yaml"
        ),
        None => false,
    }
}

fn parse_languages(values: &[String]) -> anyhow::Result<Vec<FlowLanguage>> {
    let mut langs = Vec::new();
    for value in values {
        let v = value.trim().to_lowercase();
        let lang = match v.as_str() {
            "rust" => FlowLanguage::Rust,
            "typescript" | "ts" => FlowLanguage::TypeScript,
            "javascript" | "js" => FlowLanguage::JavaScript,
            "python" | "py" => FlowLanguage::Python,
            _ => {
                return Err(anyhow!(
                    "Unknown language `{}`. Use rust, typescript, javascript, python.",
                    value
                ))
            }
        };
        langs.push(lang);
    }
    if langs.is_empty() {
        Err(anyhow!("No valid languages specified."))
    } else {
        Ok(langs)
    }
}

fn print_flow_check_report(report: &FlowCheckReport) {
    println!(
        "{} {} file(s), {} error(s), {} warning(s)",
        style("Flow check:").bold(),
        report.files.len(),
        report.error_count,
        report.warning_count
    );
    for file in &report.files {
        if file.issues.is_empty() {
            continue;
        }
        println!("  {}", style(&file.path).bold());
        for issue in &file.issues {
            let label = match issue.severity {
                IssueSeverity::Error => style("error").red(),
                IssueSeverity::Warning => style("warn").yellow(),
            };
            let field = issue
                .field
                .as_ref()
                .map(|f| format!(" ({})", f))
                .unwrap_or_default();
            println!("    [{}] {}{}", label, issue.message, field);
        }
    }
}

fn print_flow_audit_report(output: &FlowAuditOutput) {
    if let Some(flow_check) = &output.flow_check {
        print_flow_check_report(flow_check);
        println!();
    }
    let summary = &output.audit.summary;
    println!(
        "{} {} files scanned, {} findings",
        style("Flow audit:").bold(),
        summary.files_scanned,
        summary.findings
    );
    if !summary.by_category.is_empty() {
        let mut cats: Vec<_> = summary.by_category.iter().collect();
        cats.sort_by(|a, b| b.1.cmp(a.1));
        let cat_text = cats
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("  categories: {}", cat_text);
    }
    if !summary.by_language.is_empty() {
        let mut langs: Vec<_> = summary.by_language.iter().collect();
        langs.sort_by(|a, b| b.1.cmp(a.1));
        let lang_text = langs
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("  languages: {}", lang_text);
    }

    if output.audit.findings.is_empty() {
        println!("  {}", style("no findings").green());
        return;
    }

    println!();
    for finding in output.audit.findings.iter().take(20) {
        let line = finding
            .line
            .map(|l| format!(":{}:", l))
            .unwrap_or_else(|| ":".into());
        println!(
            "  [{}] {}{} {}",
            style(format!("{:?}", finding.category)).yellow(),
            finding.path,
            line,
            finding.message
        );
    }
    if output.audit.findings.len() > 20 {
        println!(
            "  ...and {} more (use --json for full output)",
            output.audit.findings.len() - 20
        );
    }
}

fn write_json(path: &Path, payload: &impl Serialize) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let json = serde_json::to_string_pretty(payload)?;
    fs::write(path, json)?;
    Ok(())
}

fn write_text(path: &Path, content: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(path, content)?;
    Ok(())
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum BlueprintJsonlRecord<'a> {
    Node {
        #[serde(flatten)]
        node: &'a dwg_core::blueprint::BlueprintNode,
    },
    Edge {
        #[serde(flatten)]
        edge: &'a dwg_core::blueprint::BlueprintEdge,
    },
    Stats {
        #[serde(flatten)]
        stats: &'a dwg_core::blueprint::BlueprintStats,
    },
    Error {
        #[serde(flatten)]
        error: &'a dwg_core::blueprint::BlueprintError,
    },
    Orphan {
        path: &'a str,
        language: String,
        lines: u32,
        reason: &'static str,
    },
}

fn blueprint_to_jsonl(report: &BlueprintReport) -> anyhow::Result<String> {
    use std::collections::HashSet;

    let mut out = String::new();

    // Collect nodes with inbound edges
    let mut has_inbound: HashSet<&str> = HashSet::new();
    for edge in &report.edges {
        if edge.resolved {
            if let Some(ref to) = edge.to {
                has_inbound.insert(to.as_str());
            }
        }
    }

    // Output nodes
    for node in &report.nodes {
        out.push_str(&serde_json::to_string(&BlueprintJsonlRecord::Node {
            node,
        })?);
        out.push('\n');
    }

    // Output edges
    for edge in &report.edges {
        out.push_str(&serde_json::to_string(&BlueprintJsonlRecord::Edge {
            edge,
        })?);
        out.push('\n');
    }

    // Output orphans (files with no inbound edges - potential dead code or entry points)
    for node in &report.nodes {
        if !has_inbound.contains(node.path.as_str()) {
            out.push_str(&serde_json::to_string(&BlueprintJsonlRecord::Orphan {
                path: &node.path,
                language: serde_json::to_string(&node.language)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_string(),
                lines: node.lines,
                reason: "no_inbound_edges",
            })?);
            out.push('\n');
        }
    }

    // Output stats
    out.push_str(&serde_json::to_string(&BlueprintJsonlRecord::Stats {
        stats: &report.stats,
    })?);
    out.push('\n');

    // Output errors
    for error in &report.errors {
        out.push_str(&serde_json::to_string(&BlueprintJsonlRecord::Error {
            error,
        })?);
        out.push('\n');
    }

    Ok(out)
}

fn slugify_kebab(input: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in input.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

fn render_flow_template(
    name: &str,
    entrypoint: &str,
    language: Option<&str>,
    indirection_budget: Option<usize>,
) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("name: \"{}\"\n", escape_yaml_string(name)));
    out.push_str(&format!(
        "entrypoint: \"{}\"\n",
        escape_yaml_string(entrypoint)
    ));
    out.push_str("inputs:\n  - \"<input>\"\n");
    out.push_str("outputs:\n  - \"<output>\"\n");
    out.push_str("side_effects:\n  - \"<side effect>\"\n");
    out.push_str("failure_modes:\n  - \"<failure mode>\"\n");
    out.push_str("observability:\n  - \"<signal/log/metric>\"\n");
    out.push_str("steps:\n");
    out.push_str("  - \"<step 1>\"\n  - \"<step 2>\"\n  - \"<step 3>\"\n");
    out.push_str("invariants:\n");
    out.push_str("  - \"<invariant 1>\"\n  - \"<invariant 2>\"\n  - \"<invariant 3>\"\n");
    if let Some(budget) = indirection_budget {
        out.push_str(&format!("indirection_budget: {budget}\n"));
    }
    out.push_str("justifications:\n");
    out.push_str("  - item: \"<new thing you might add>\"\n");
    out.push_str("    reason: \"policy\"\n");
    out.push_str("    evidence: \"<why does it exist?>\"\n");
    out.push_str("tags: []\n");
    out.push_str("owners: []\n");
    if let Some(lang) = language {
        if !lang.trim().is_empty() {
            out.push_str(&format!("language: \"{}\"\n", escape_yaml_string(lang)));
        }
    }
    out.push_str("---\n\n");
    out.push_str(
        "When in doubt, produce something people can point at.\n\nThis file is that artifact.\n",
    );
    out
}

fn escape_yaml_string(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

fn render_flow_proposal(flow_check: &Option<FlowCheckReport>, audit: &FlowAuditReport) -> String {
    let mut out = String::new();
    out.push_str("# ToneGuard Flow Proposal\n\n");
    out.push_str("A concrete review artifact: flow spec checks + static entropy findings.\n\n");
    out.push_str("> **For AI Agents**: Each finding includes machine-readable fix instructions in JSON format.\n\n");
    out.push_str("## Flow checks\n\n");
    match flow_check {
        Some(report) => {
            out.push_str(&format!("- Files: {}\n", report.files.len()));
            out.push_str(&format!("- Errors: {}\n", report.error_count));
            out.push_str(&format!("- Warnings: {}\n", report.warning_count));
            out.push('\n');

            for file in &report.files {
                if file.issues.is_empty() {
                    continue;
                }
                out.push_str(&format!("### {}\n\n", file.path));
                for issue in &file.issues {
                    out.push_str(&format!(
                        "- {:?} {}{}\n",
                        issue.severity,
                        issue
                            .field
                            .as_ref()
                            .map(|f| format!("({f}) "))
                            .unwrap_or_default(),
                        issue.message
                    ));
                }
                out.push('\n');
            }
        }
        None => {
            out.push_str("- Skipped (`--no-flow-checks`).\n\n");
        }
    }

    out.push_str("## Audit summary\n\n");
    out.push_str(&format!(
        "- Files scanned: {}\n",
        audit.summary.files_scanned
    ));
    out.push_str(&format!("- Findings: {}\n", audit.summary.findings));
    if !audit.summary.by_category.is_empty() {
        out.push_str("- By category:\n");
        for (key, value) in &audit.summary.by_category {
            out.push_str(&format!("  - {key}: {value}\n"));
        }
    }
    if !audit.summary.by_language.is_empty() {
        out.push_str("- By language:\n");
        for (key, value) in &audit.summary.by_language {
            out.push_str(&format!("  - {key}: {value}\n"));
        }
    }
    out.push('\n');

    if audit.findings.is_empty() {
        out.push_str("## Findings\n\nNo findings. Code is clean.\n");
        return out;
    }

    out.push_str("## Findings\n\n");
    let mut grouped: BTreeMap<String, Vec<&dwg_core::arch::FlowFinding>> = BTreeMap::new();
    for finding in &audit.findings {
        grouped
            .entry(format!("{:?}", finding.category))
            .or_default()
            .push(finding);
    }

    for (category, items) in grouped {
        out.push_str(&format!("### {} ({})\n\n", category, items.len()));
        for finding in items {
            let location = finding.line.map(|l| format!(":{}", l)).unwrap_or_default();

            // Header with location
            out.push_str(&format!(
                "#### [{:?}] `{}{}`\n\n",
                finding.severity, finding.path, location,
            ));

            // What: the problem
            out.push_str(&format!("**What**: {}\n\n", finding.message));

            // Fix options (human-readable)
            if let Some(fix) = &finding.fix_instructions {
                out.push_str("**Fix options**:\n");
                out.push_str(&format!(
                    "1. **{}**: {}\n",
                    capitalize_first(&fix.action),
                    fix.description
                ));
                if let Some(alt) = &fix.alternative {
                    out.push_str(&format!("2. **Justify**: {}\n", alt));
                }
                out.push('\n');

                // Machine-readable JSON for AI agents
                out.push_str("**For AI agents**:\n\n");
                out.push_str("```json\n");
                let json_fix = serde_json::json!({
                    "file": finding.path,
                    "line": finding.line,
                    "action": fix.action,
                    "find": fix.find_pattern,
                    "replace": fix.replace_pattern,
                    "alternative": fix.alternative
                });
                if let Ok(json_str) = serde_json::to_string_pretty(&json_fix) {
                    out.push_str(&json_str);
                }
                out.push_str("\n```\n\n");
            }

            // Evidence
            if !finding.evidence.is_empty() {
                out.push_str("**Evidence**:\n");
                for ev in &finding.evidence {
                    out.push_str(&format!("- {ev}\n"));
                }
                out.push('\n');
            }

            out.push_str("---\n\n");
        }
    }

    out.push_str("## How to use this document\n\n");
    out.push_str("### For humans\n");
    out.push_str("1. Review each finding above\n");
    out.push_str("2. Either fix the issue OR justify it in a flow spec\n");
    out.push_str("3. Re-run `dwg flow propose` to verify fixes\n\n");
    out.push_str("### For AI agents (Claude/Codex)\n");
    out.push_str("1. Parse the JSON blocks for each finding\n");
    out.push_str("2. Apply the suggested `find`/`replace` patterns\n");
    out.push_str(
        "3. If justification is more appropriate, add to `flows/*.md` under `justifications:`\n\n",
    );
    out.push_str("### Justification reasons\n");
    out.push_str("- `variation`: The abstraction exists because implementations will differ\n");
    out.push_str("- `isolation`: The wrapper isolates callers from implementation changes\n");
    out.push_str("- `reuse`: The duplicated code is intentionally repeated (not DRY by design)\n");
    out.push_str("- `policy`: Business/security policy requires this structure\n");
    out.push_str(
        "- `volatility`: This code changes frequently; abstraction reduces blast radius\n",
    );
    out
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
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

// ─────────────────────────────────────────────────────────────────────────────
// organize subcommand
// ─────────────────────────────────────────────────────────────────────────────

fn run_organize(args: OrganizeArgs) -> anyhow::Result<()> {
    let root = if args.paths.is_empty() {
        PathBuf::from(".")
    } else {
        args.paths[0].clone()
    };

    let root = fs::canonicalize(&root).unwrap_or(root);

    // Build config (merge organize + repo ignore rules)
    let (cfg, _config_root) = load_config(&args.config)?;
    let mut config = cfg.organize_rules;
    if !cfg.repo_rules.ignore_globs.is_empty() {
        let mut merged = config.ignore_globs.clone();
        merged.extend(cfg.repo_rules.ignore_globs);
        merged.sort();
        merged.dedup();
        config.ignore_globs = merged;
    }
    if let Some(kb) = config.data_file_min_kb {
        config.data_file_min_size = kb * 1024;
    }
    if let Some(kb) = args.data_min_kb {
        config.data_file_min_size = kb * 1024;
    }
    config.check_git_status = !args.no_git;

    // Run analysis
    let report = analyze_organization(&root, &config)?;

    // Generate output based on options
    if let Some(agent) = &args.prompt_for {
        let prompt = generate_organize_prompt(&report, agent);
        if let Some(out) = &args.out {
            fs::write(out, &prompt)?;
            println!("Organization prompt written to {}", out.display());
        } else {
            println!("{}", prompt);
        }
    } else if args.json {
        let json = serde_json::to_string_pretty(&report)?;
        if let Some(out) = &args.out {
            fs::write(out, &json)?;
            println!("Organization report written to {}", out.display());
        } else {
            println!("{}", json);
        }
    } else {
        // Human-readable output
        print_organize_report(&report);
        if let Some(out) = &args.out {
            let json = serde_json::to_string_pretty(&report)?;
            fs::write(out, &json)?;
            println!("\nReport written to {}", out.display());
        }
    }

    Ok(())
}

fn print_organize_report(report: &OrganizationReport) {
    use console::style;

    println!();
    println!(
        "{}",
        style("Repository Organization Analysis").bold().cyan()
    );
    println!("{}", style("═".repeat(50)).dim());

    // Repo type
    println!(
        "\n{}: {:?} (confidence: {:.0}%)",
        style("Detected Type").bold(),
        report.repo_type.kind,
        report.repo_type.confidence * 100.0
    );
    if !report.repo_type.indicators.is_empty() {
        println!("  Indicators: {}", report.repo_type.indicators.join(", "));
    }
    if !report.repo_type.expected_structure.is_empty() {
        println!(
            "  Expected structure: {}",
            report.repo_type.expected_structure.join(", ")
        );
    }

    println!(
        "\n{}: {}",
        style("Files Scanned").bold(),
        report.files_scanned
    );
    println!(
        "{}: {}",
        style("Issues Found").bold(),
        style(report.findings.len()).yellow()
    );

    if report.findings.is_empty() {
        println!(
            "\n{}",
            style("No organizational issues found. Repository is well-organized.").green()
        );
        return;
    }

    // Summary by type
    println!("\n{}", style("Summary by Issue Type:").bold());
    for (issue_type, count) in &report.summary {
        println!("  {}: {}", issue_type.replace('_', " "), count);
    }

    // Findings grouped by type
    println!("\n{}", style("Findings:").bold());

    // Group findings
    let mut by_type: std::collections::BTreeMap<
        String,
        Vec<&dwg_core::organize::OrganizationFinding>,
    > = std::collections::BTreeMap::new();
    for finding in &report.findings {
        let key = format!("{:?}", finding.issue);
        by_type.entry(key).or_default().push(finding);
    }

    for (issue_type, findings) in by_type {
        println!(
            "\n  {} ({}):",
            style(&issue_type).yellow().bold(),
            findings.len()
        );
        for finding in findings.iter().take(10) {
            let path_display = finding
                .path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| finding.path.display().to_string());

            let action = match finding.suggested_action {
                dwg_core::organize::Action::Move => style("MOVE").blue(),
                dwg_core::organize::Action::Delete => style("DELETE").red(),
                dwg_core::organize::Action::Archive => style("ARCHIVE").magenta(),
                dwg_core::organize::Action::Gitignore => style("GITIGNORE").dim(),
            };

            println!("    {} {} - {}", action, path_display, finding.reason);

            if let Some(target) = &finding.target_path {
                println!("      → {}", style(target.display()).dim());
            }
        }
        if findings.len() > 10 {
            println!(
                "    {} more...",
                style(format!("... and {}", findings.len() - 10)).dim()
            );
        }
    }

    println!();
    println!(
        "{}",
        style("Use --prompt-for cursor|claude|codex to generate AI fix instructions").dim()
    );
}
