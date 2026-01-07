//! Repo Organizer: Detects structural mess and suggests organization.
//!
//! This module helps prevent "repo entropy" by detecting:
//! - Misplaced files (scripts in wrong directories)
//! - Legacy/backup files (*_v1, *.bak, *_old)
//! - Data files scattered in source directories
//! - Untracked experiment files

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

const OUTPUT_DIR_HINTS: &[&str] = &[
    "output",
    "outputs",
    "out",
    "results",
    "artifacts",
    "generated",
    "exports",
    "export",
    "tmp",
    "temp",
];

const EXPERIMENT_NAME_HINTS: &[&str] = &[
    "test",
    "scratch",
    "temp",
    "debug",
    "poc",
    "experiment",
    "analysis",
];

/// The detected type/flavor of a repository.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RepoKind {
    /// React/Vue/Svelte frontend app
    Frontend,
    /// Python package or data science project
    Python,
    /// Monorepo with multiple packages
    Monorepo,
    /// Rust workspace
    RustWorkspace,
    /// Mixed or unidentified
    Mixed,
}

impl Default for RepoKind {
    fn default() -> Self {
        RepoKind::Mixed
    }
}

/// Result of repo type detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoType {
    /// The detected repo kind
    pub kind: RepoKind,
    /// Confidence score (0.0 to 1.0)
    pub confidence: f32,
    /// Indicator files/patterns that led to this detection
    pub indicators: Vec<String>,
    /// Expected directory structure for this repo type
    pub expected_structure: Vec<String>,
}

/// Types of organizational issues.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum IssueKind {
    /// File type is in the wrong directory
    Misplaced,
    /// Legacy/backup file (*_v1, *.bak, *_old)
    Legacy,
    /// Large data file in source directory
    DataInSource,
    /// Untracked file that looks like an experiment
    UntrackedExperiment,
    /// Near-duplicate files
    Duplicate,
}

/// Suggested action for a finding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    /// Move the file to a different location
    Move,
    /// Delete the file
    Delete,
    /// Archive to an archive/ directory
    Archive,
    /// Add to .gitignore
    Gitignore,
}

/// An organizational finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrganizationFinding {
    /// Path to the problematic file
    pub path: PathBuf,
    /// Type of issue
    pub issue: IssueKind,
    /// Suggested action
    pub suggested_action: Action,
    /// Target path for move/archive actions
    pub target_path: Option<PathBuf>,
    /// Human-readable reason
    pub reason: String,
    /// Size in bytes (for data files)
    pub size_bytes: Option<u64>,
    /// Is the file tracked in git?
    pub git_tracked: bool,
}

/// Configuration for the organizer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OrganizeConfig {
    /// Minimum file size (bytes) to flag as "data file"
    pub data_file_min_size: u64,
    /// Optional KB override from config (normalized to bytes on load)
    pub data_file_min_kb: Option<u64>,
    /// Extensions considered as data files
    pub data_extensions: Vec<String>,
    /// Extensions considered as scripts
    pub script_extensions: Vec<String>,
    /// Patterns that indicate legacy/backup files
    pub legacy_patterns: Vec<String>,
    /// Directories where scripts are expected
    pub script_directories: Vec<String>,
    /// Directories where data files are expected
    pub data_directories: Vec<String>,
    /// Globs to ignore
    pub ignore_globs: Vec<String>,
    /// Whether to check git status
    pub check_git_status: bool,
}

impl Default for OrganizeConfig {
    fn default() -> Self {
        Self {
            data_file_min_size: 100 * 1024, // 100KB
            data_file_min_kb: None,
            data_extensions: vec![
                "csv".into(),
                "xlsx".into(),
                "xls".into(),
                "json".into(),
                "parquet".into(),
                "sqlite".into(),
                "db".into(),
            ],
            script_extensions: vec![
                "py".into(),
                "sh".into(),
                "bash".into(),
                "ps1".into(),
                "rb".into(),
                "pl".into(),
            ],
            legacy_patterns: vec![
                "*_v[0-9]*".into(),
                "*_old*".into(),
                "*.bak".into(),
                "*~".into(),
                "*.orig".into(),
                "*_backup*".into(),
                "*_copy*".into(),
                "Copy of *".into(),
            ],
            script_directories: vec![
                "scripts".into(),
                "bin".into(),
                "tools".into(),
                "utils".into(),
                "scratch".into(),
                "experiments".into(),
                "analysis".into(),
                "poc".into(),
            ],
            data_directories: vec![
                "data".into(),
                "assets".into(),
                "fixtures".into(),
                "testdata".into(),
                "public".into(),
                "output".into(),
                "outputs".into(),
                "reports".into(),
                "artifacts".into(),
                "exports".into(),
                "export".into(),
            ],
            ignore_globs: vec![
                "node_modules/**".into(),
                ".git/**".into(),
                "target/**".into(),
                "dist/**".into(),
                "build/**".into(),
                "__pycache__/**".into(),
                ".venv/**".into(),
                "venv/**".into(),
            ],
            check_git_status: true,
        }
    }
}

/// Full organization report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrganizationReport {
    /// Detected repo type
    pub repo_type: RepoType,
    /// List of findings
    pub findings: Vec<OrganizationFinding>,
    /// Summary counts by issue type
    pub summary: HashMap<String, usize>,
    /// Total files scanned
    pub files_scanned: usize,
}

/// Detect the type of repository based on marker files.
pub fn detect_repo_type(root: &Path) -> RepoType {
    let mut indicators = Vec::new();
    let mut kind = RepoKind::Mixed;
    let mut confidence = 0.3;
    let mut expected = Vec::new();

    // Check for frontend markers
    let package_json = root.join("package.json");
    let has_react = root.join("src/App.tsx").exists()
        || root.join("src/App.jsx").exists()
        || root.join("src/App.vue").exists()
        || root.join("src/App.svelte").exists();
    let has_package = package_json.exists();

    if has_package && has_react {
        kind = RepoKind::Frontend;
        confidence = 0.9;
        indicators.push("package.json + src/App.{tsx,jsx,vue,svelte}".into());
        expected = vec![
            "src/".into(),
            "public/".into(),
            "dist/".into(),
            "scripts/".into(),
        ];
    }

    // Check for Python markers
    let has_pyproject = root.join("pyproject.toml").exists();
    let has_setup = root.join("setup.py").exists();
    let has_requirements = root.join("requirements.txt").exists();
    let has_notebooks = WalkDir::new(root)
        .max_depth(2)
        .into_iter()
        .filter_map(|e| e.ok())
        .any(|e| {
            e.path()
                .extension()
                .map_or(false, |ext| ext == "ipynb")
        });

    if has_pyproject || has_setup {
        if kind == RepoKind::Mixed {
            kind = RepoKind::Python;
            confidence = 0.85;
            indicators.push("pyproject.toml or setup.py".into());
            expected = vec![
                "src/".into(),
                "tests/".into(),
                "docs/".into(),
                "scripts/".into(),
            ];
        }
    } else if has_requirements && has_notebooks {
        if kind == RepoKind::Mixed {
            kind = RepoKind::Python;
            confidence = 0.8;
            indicators.push("requirements.txt + notebooks".into());
            expected = vec![
                "notebooks/".into(),
                "data/".into(),
                "scripts/".into(),
                "outputs/".into(),
            ];
        }
    }

    // Check for monorepo markers
    let has_pnpm_workspace = root.join("pnpm-workspace.yaml").exists();
    let has_lerna = root.join("lerna.json").exists();
    let has_nx = root.join("nx.json").exists();
    let has_turbo = root.join("turbo.json").exists();

    if has_pnpm_workspace || has_lerna || has_nx || has_turbo {
        kind = RepoKind::Monorepo;
        confidence = 0.95;
        if has_pnpm_workspace {
            indicators.push("pnpm-workspace.yaml".into());
        }
        if has_lerna {
            indicators.push("lerna.json".into());
        }
        if has_nx {
            indicators.push("nx.json".into());
        }
        if has_turbo {
            indicators.push("turbo.json".into());
        }
        expected = vec![
            "packages/".into(),
            "apps/".into(),
            "libs/".into(),
        ];
    }

    // Check for Rust workspace
    let cargo_toml = root.join("Cargo.toml");
    if cargo_toml.exists() {
        if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
            if content.contains("[workspace]") {
                kind = RepoKind::RustWorkspace;
                confidence = 0.95;
                indicators.push("Cargo.toml with [workspace]".into());
                expected = vec![
                    "crates/".into(),
                    "bins/".into(),
                    "cli/".into(),
                    "core/".into(),
                ];
            }
        }
    }

    RepoType {
        kind,
        confidence,
        indicators,
        expected_structure: expected,
    }
}

/// Check if a file is tracked in git.
fn is_git_tracked(root: &Path, file: &Path) -> bool {
    let relative = file.strip_prefix(root).unwrap_or(file);
    let output = Command::new("git")
        .args(["ls-files", "--error-unmatch"])
        .arg(relative)
        .current_dir(root)
        .output();

    match output {
        Ok(out) => out.status.success(),
        Err(_) => true, // Assume tracked if git command fails
    }
}

/// Get the size of a file.
fn file_size(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

/// Check if a filename matches legacy patterns.
fn is_legacy_file(filename: &str, patterns: &[String]) -> Option<String> {
    let lower = filename.to_lowercase();

    // Simple pattern matching
    for pattern in patterns {
        let pat_lower = pattern.to_lowercase();

        // Handle *_v[0-9]* pattern
        if pat_lower.contains("[0-9]") {
            let re_pattern = pat_lower
                .replace("*", ".*")
                .replace("[0-9]", "[0-9]");
            if let Ok(re) = regex::Regex::new(&format!("^{}$", re_pattern)) {
                if re.is_match(&lower) {
                    return Some(format!("matches pattern: {}", pattern));
                }
            }
        } else if pat_lower.starts_with('*') && pat_lower.ends_with('*') {
            // *_old* pattern
            let inner = &pat_lower[1..pat_lower.len() - 1];
            if lower.contains(inner) {
                return Some(format!("matches pattern: {}", pattern));
            }
        } else if pat_lower.starts_with('*') {
            // *.bak pattern
            let suffix = &pat_lower[1..];
            if lower.ends_with(suffix) {
                return Some(format!("matches pattern: {}", pattern));
            }
        } else if pat_lower.ends_with('*') {
            // Copy of * pattern
            let prefix = &pat_lower[..pat_lower.len() - 1];
            if lower.starts_with(prefix) {
                return Some(format!("matches pattern: {}", pattern));
            }
        }
    }

    None
}

fn output_dir_prefix(relative: &Path) -> Option<PathBuf> {
    let parent = relative.parent().unwrap_or(relative);
    let mut prefix = PathBuf::new();
    for comp in parent.components() {
        if let Component::Normal(os_str) = comp {
            let name = os_str.to_string_lossy().to_lowercase();
            prefix.push(os_str);
            if OUTPUT_DIR_HINTS.iter().any(|hint| hint == &name) {
                return Some(prefix);
            }
        }
    }
    None
}

fn looks_experimental(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    EXPERIMENT_NAME_HINTS
        .iter()
        .any(|hint| lower.contains(hint))
}

/// Check if a path is in a "source" directory (not data/scripts).
fn is_in_source_dir(path: &Path, _repo_type: &RepoType, config: &OrganizeConfig) -> bool {
    let path_str = path.to_string_lossy().to_lowercase();

    // If in a known script or data directory, not "source"
    for dir in &config.script_directories {
        if path_str.contains(&format!("/{}/", dir.to_lowercase()))
            || path_str.contains(&format!("\\{}\\", dir.to_lowercase()))
        {
            return false;
        }
    }
    for dir in &config.data_directories {
        if path_str.contains(&format!("/{}/", dir.to_lowercase()))
            || path_str.contains(&format!("\\{}\\", dir.to_lowercase()))
        {
            return false;
        }
    }

    // Check if in typical source directories
    let source_indicators = ["src/", "lib/", "app/", "ui/", "components/", "pages/"];
    for indicator in source_indicators {
        if path_str.contains(indicator) || path_str.contains(&indicator.replace('/', "\\")) {
            return true;
        }
    }

    // Root level is considered "source" for misplacement purposes
    true
}

/// Build a GlobSet from patterns.
fn build_glob_set(patterns: &[String]) -> Option<GlobSet> {
    if patterns.is_empty() {
        return None;
    }
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        // Add ** prefix if not present
        let p = if pattern.contains('/') || pattern.contains('\\') {
            pattern.clone()
        } else {
            format!("**/{}", pattern)
        };
        if let Ok(glob) = Glob::new(&p) {
            builder.add(glob);
        }
    }
    builder.build().ok()
}

/// Analyze a repository for organizational issues.
pub fn analyze_organization(root: &Path, config: &OrganizeConfig) -> anyhow::Result<OrganizationReport> {
    let repo_type = detect_repo_type(root);
    let mut findings = Vec::new();
    let mut files_scanned = 0;

    // Build ignore globs
    let ignore_set = build_glob_set(&config.ignore_globs);

    // Data file extensions as a set
    let data_exts: HashSet<_> = config
        .data_extensions
        .iter()
        .map(|e| e.to_lowercase())
        .collect();

    // Script extensions as a set
    let script_exts: HashSet<_> = config
        .script_extensions
        .iter()
        .map(|e| e.to_lowercase())
        .collect();

    // Walk the directory
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        // Skip directories
        if path.is_dir() {
            continue;
        }

        // Check against ignore globs
        let relative = path.strip_prefix(root).unwrap_or(path);
        if let Some(ref gs) = ignore_set {
            if gs.is_match(relative) {
                continue;
            }
        }

        files_scanned += 1;

        let filename = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let extension = path
            .extension()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        let git_tracked = if config.check_git_status {
            is_git_tracked(root, path)
        } else {
            true
        };

        if !git_tracked {
            if let Some(output_dir) = output_dir_prefix(relative) {
                findings.push(OrganizationFinding {
                    path: path.to_path_buf(),
                    issue: IssueKind::UntrackedExperiment,
                    suggested_action: Action::Gitignore,
                    target_path: Some(output_dir),
                    reason: "untracked file in output directory".into(),
                    size_bytes: file_size(path),
                    git_tracked,
                });
                continue;
            }
        }

        // Check for legacy/backup files
        if let Some(reason) = is_legacy_file(&filename, &config.legacy_patterns) {
            findings.push(OrganizationFinding {
                path: path.to_path_buf(),
                issue: IssueKind::Legacy,
                suggested_action: if filename.ends_with(".bak")
                    || filename.ends_with('~')
                    || filename.ends_with(".orig")
                {
                    Action::Delete
                } else {
                    Action::Archive
                },
                target_path: Some(root.join("archive").join(&filename)),
                reason,
                size_bytes: file_size(path),
                git_tracked,
            });
            continue; // Don't double-count
        }

        // Check for data files in source directories
        if data_exts.contains(&extension) {
            let size = file_size(path).unwrap_or(0);
            if size >= config.data_file_min_size && is_in_source_dir(path, &repo_type, config) {
                findings.push(OrganizationFinding {
                    path: path.to_path_buf(),
                    issue: IssueKind::DataInSource,
                    suggested_action: Action::Move,
                    target_path: Some(root.join("data").join(&filename)),
                    reason: format!(
                        "{} data file ({:.1} KB) in source directory",
                        extension.to_uppercase(),
                        size as f64 / 1024.0
                    ),
                    size_bytes: Some(size),
                    git_tracked,
                });
                continue;
            }
        }

        // Check for scripts in wrong directories (e.g., Python scripts in ui/)
        if script_exts.contains(&extension) {
            let in_expected = config
                .script_directories
                .iter()
                .any(|d| relative.starts_with(d) || relative.to_string_lossy().contains(&format!("/{}/", d)));

            // Check if this is in a "wrong" directory for scripts
            let in_ui = relative.to_string_lossy().contains("ui/")
                || relative.to_string_lossy().contains("ui\\")
                || relative.to_string_lossy().contains("src/components")
                || relative.to_string_lossy().contains("src\\components");

            if in_ui && !in_expected {
                findings.push(OrganizationFinding {
                    path: path.to_path_buf(),
                    issue: IssueKind::Misplaced,
                    suggested_action: Action::Move,
                    target_path: Some(root.join("scripts").join(&filename)),
                    reason: format!(
                        "{} script in UI/component directory",
                        extension.to_uppercase()
                    ),
                    size_bytes: file_size(path),
                    git_tracked,
                });
                continue;
            }
        }

        // Check for untracked experiments
        if !git_tracked && script_exts.contains(&extension) {
            // Heuristics for "experiment" scripts
            let looks_experimental = looks_experimental(&filename);

            if looks_experimental {
                findings.push(OrganizationFinding {
                    path: path.to_path_buf(),
                    issue: IssueKind::UntrackedExperiment,
                    suggested_action: Action::Move,
                    target_path: Some(root.join("scratch").join(&filename)),
                    reason: "untracked file with experimental naming".into(),
                    size_bytes: file_size(path),
                    git_tracked,
                });
            }
        }
    }

    // Build summary
    let mut summary = HashMap::new();
    for finding in &findings {
        let key = match finding.issue {
            IssueKind::Misplaced => "misplaced",
            IssueKind::Legacy => "legacy",
            IssueKind::DataInSource => "data_in_source",
            IssueKind::UntrackedExperiment => "untracked_experiment",
            IssueKind::Duplicate => "duplicate",
        };
        *summary.entry(key.to_string()).or_insert(0) += 1;
    }

    Ok(OrganizationReport {
        repo_type,
        findings,
        summary,
        files_scanned,
    })
}

/// Generate an AI prompt for fixing organization issues.
pub fn generate_organize_prompt(report: &OrganizationReport, agent: &str) -> String {
    let mut prompt = String::new();

    prompt.push_str("# Repository Organization Issues\n\n");
    prompt.push_str(&format!(
        "Detected repo type: {:?} (confidence: {:.0}%)\n",
        report.repo_type.kind,
        report.repo_type.confidence * 100.0
    ));
    prompt.push_str(&format!("Files scanned: {}\n", report.files_scanned));
    prompt.push_str(&format!("Issues found: {}\n\n", report.findings.len()));

    if report.findings.is_empty() {
        prompt.push_str("No organizational issues found. The repository is well-organized.\n");
        return prompt;
    }

    prompt.push_str("## Issues to Fix\n\n");

    // Group by issue type
    let mut by_type: HashMap<&IssueKind, Vec<&OrganizationFinding>> = HashMap::new();
    for finding in &report.findings {
        by_type.entry(&finding.issue).or_default().push(finding);
    }

    for (issue_type, findings) in by_type {
        let type_name = match issue_type {
            IssueKind::Misplaced => "Misplaced Files",
            IssueKind::Legacy => "Legacy/Backup Files",
            IssueKind::DataInSource => "Data Files in Source",
            IssueKind::UntrackedExperiment => "Untracked Experiments",
            IssueKind::Duplicate => "Duplicate Files",
        };

        prompt.push_str(&format!("### {} ({})\n\n", type_name, findings.len()));

        for finding in findings {
            let action = match finding.suggested_action {
                Action::Move => "MOVE",
                Action::Delete => "DELETE",
                Action::Archive => "ARCHIVE",
                Action::Gitignore => "GITIGNORE",
            };

            prompt.push_str(&format!("- `{}`\n", finding.path.display()));
            prompt.push_str(&format!("  - Reason: {}\n", finding.reason));
            prompt.push_str(&format!("  - Action: {}", action));
            if let Some(target) = &finding.target_path {
                prompt.push_str(&format!(" to `{}`", target.display()));
            }
            prompt.push('\n');
        }
        prompt.push('\n');
    }

    prompt.push_str("## Instructions\n\n");

    match agent {
        "cursor" => {
            prompt.push_str("Please reorganize this repository by:\n");
            prompt.push_str("1. Creating any missing directories (scripts/, data/, archive/)\n");
            prompt.push_str("2. Moving files to their suggested locations\n");
            prompt.push_str("3. Deleting backup files that are safe to remove\n");
            prompt.push_str("4. Updating any imports/references affected by the moves\n");
        }
        "claude" => {
            prompt.push_str("Please help reorganize this repository. For each issue:\n");
            prompt.push_str("1. Confirm the suggested action is appropriate\n");
            prompt.push_str("2. Execute the move/delete/archive operation\n");
            prompt.push_str("3. Update any code references if needed\n");
        }
        "codex" => {
            prompt.push_str("Execute the following reorganization:\n\n");
            prompt.push_str("```bash\n");
            prompt.push_str("# Create directories\n");
            prompt.push_str("mkdir -p scripts data archive scratch\n\n");

            for finding in &report.findings {
                match finding.suggested_action {
                    Action::Move | Action::Archive => {
                        if let Some(target) = &finding.target_path {
                            prompt.push_str(&format!(
                                "mv \"{}\" \"{}\"\n",
                                finding.path.display(),
                                target.display()
                            ));
                        }
                    }
                    Action::Delete => {
                        prompt.push_str(&format!("rm \"{}\"\n", finding.path.display()));
                    }
                    Action::Gitignore => {
                        let target = finding
                            .target_path
                            .as_ref()
                            .unwrap_or(&finding.path);
                        prompt.push_str(&format!(
                            "echo \"{}\" >> .gitignore\n",
                            target.display()
                        ));
                    }
                }
            }
            prompt.push_str("```\n");
        }
        _ => {}
    }

    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn test_is_legacy_file() {
        let patterns = vec![
            "*_v[0-9]*".into(),
            "*_old*".into(),
            "*.bak".into(),
        ];

        assert!(is_legacy_file("file_v1.py", &patterns).is_some());
        assert!(is_legacy_file("file_v2.txt", &patterns).is_some());
        assert!(is_legacy_file("script_old.sh", &patterns).is_some());
        assert!(is_legacy_file("backup.bak", &patterns).is_some());
        assert!(is_legacy_file("normal_file.py", &patterns).is_none());
    }

    #[test]
    fn test_detect_repo_type_rust() {
        // This would need a mock filesystem, but we can at least test the structure
        let report = RepoType {
            kind: RepoKind::RustWorkspace,
            confidence: 0.95,
            indicators: vec!["Cargo.toml with [workspace]".into()],
            expected_structure: vec!["crates/".into(), "cli/".into()],
        };
        assert_eq!(report.kind, RepoKind::RustWorkspace);
    }

    #[test]
    fn test_output_dir_prefix() {
        let path = Path::new("poc/output/results.json");
        let prefix = output_dir_prefix(path).expect("output dir prefix");
        assert_eq!(prefix, PathBuf::from("poc/output"));
    }

    #[test]
    fn test_looks_experimental() {
        assert!(looks_experimental("scratch_notes.py"));
        assert!(looks_experimental("analysis_v2.ipynb"));
        assert!(!looks_experimental("main.rs"));
    }
}
