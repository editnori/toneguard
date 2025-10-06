use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use clap::{ArgAction, Parser};
use console::style;
use dwg_core::{Analyzer, Category, Config, DocumentReport};
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
}

#[derive(Debug, Serialize)]
struct FileResult {
    path: String,
    word_count: usize,
    density_per_100_words: f32,
    category_counts: BTreeMap<Category, usize>,
    diagnostics: Vec<dwg_core::Diagnostic>,
}

#[derive(Debug, Serialize)]
struct OutputReport {
    files: Vec<FileResult>,
    total_word_count: usize,
    total_diagnostics: usize,
    density_per_100_words: f32,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let cfg = load_config(&args.config)?;
    let analyzer = Analyzer::new(cfg.clone())?;

    let mut files = collect_files(&args.paths)?;
    files.sort();

    let mut file_reports = Vec::new();
    let mut total_words = 0usize;
    let mut total_diags = 0usize;
    let mut exit_due_to_threshold = false;

    for path in files {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let report = analyzer.analyze(&content);
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

fn collect_files(paths: &[PathBuf]) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            for entry in WalkDir::new(path) {
                let entry = entry?;
                if entry.file_type().is_file() && is_supported(entry.path()) {
                    files.push(entry.path().to_path_buf());
                }
            }
        } else if path.is_file() && is_supported(path) {
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

fn load_config(path: &PathBuf) -> anyhow::Result<Config> {
    if path.exists() {
        let text = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config {}", path.display()))?;
        let value: YamlValue = serde_yaml::from_str(&text)
            .with_context(|| format!("Failed to parse YAML {}", path.display()))?;
        let cfg: Config = serde_yaml::from_value(value)
            .with_context(|| format!("Invalid config structure in {}", path.display()))?;
        Ok(cfg)
    } else {
        Ok(Config::default())
    }
}

fn print_human_report(path: &PathBuf, report: &DocumentReport, density: f32) {
    println!(
        "{} ({} words, density {:.2}/100w)",
        style(path.to_string_lossy()).bold(),
        report.word_count,
        density
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
