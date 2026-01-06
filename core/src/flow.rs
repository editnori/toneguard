use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FlowRules {
    pub indirection_budget: Option<usize>,
    pub min_steps: usize,
    pub max_steps: usize,
    pub min_invariants: usize,
    pub ignore_globs: Vec<String>,
    pub allowed_reasons: Vec<String>,
    pub duplication_min_instances: usize,
    pub duplication_min_tokens: usize,
    pub duplication_max_groups: usize,
}

impl Default for FlowRules {
    fn default() -> Self {
        Self {
            indirection_budget: Some(5),
            min_steps: 3,
            max_steps: 12,
            min_invariants: 3,
            ignore_globs: Vec::new(),
            allowed_reasons: vec![
                "variation".into(),
                "isolation".into(),
                "reuse".into(),
                "policy".into(),
                "volatility".into(),
            ],
            duplication_min_instances: 3,
            duplication_min_tokens: 80,
            duplication_max_groups: 20,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IssueSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JustificationReason {
    Variation,
    Isolation,
    Reuse,
    Policy,
    Volatility,
}

impl Default for JustificationReason {
    fn default() -> Self {
        JustificationReason::Policy
    }
}

impl JustificationReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            JustificationReason::Variation => "variation",
            JustificationReason::Isolation => "isolation",
            JustificationReason::Reuse => "reuse",
            JustificationReason::Policy => "policy",
            JustificationReason::Volatility => "volatility",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FlowJustification {
    pub item: String,
    pub reason: JustificationReason,
    pub evidence: Option<String>,
    pub tradeoff: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FlowSpec {
    pub name: String,
    pub entrypoint: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub side_effects: Vec<String>,
    pub failure_modes: Vec<String>,
    pub observability: Vec<String>,
    pub steps: Vec<String>,
    pub invariants: Vec<String>,
    pub indirection_budget: Option<usize>,
    pub justifications: Vec<FlowJustification>,
    pub tags: Vec<String>,
    pub owners: Vec<String>,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowSpecIssue {
    pub severity: IssueSeverity,
    pub field: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowSpecDocument {
    pub path: PathBuf,
    pub spec: FlowSpec,
    pub body: Option<String>,
}

impl FlowSpec {
    pub fn validate(&self, rules: &FlowRules) -> Vec<FlowSpecIssue> {
        let mut issues = Vec::new();
        if self.name.trim().is_empty() {
            issues.push(FlowSpecIssue {
                severity: IssueSeverity::Error,
                field: Some("name".into()),
                message: "Flow name is required.".into(),
            });
        }
        if self.entrypoint.trim().is_empty() {
            issues.push(FlowSpecIssue {
                severity: IssueSeverity::Error,
                field: Some("entrypoint".into()),
                message: "Entrypoint is required.".into(),
            });
        }
        if self.steps.is_empty() {
            issues.push(FlowSpecIssue {
                severity: IssueSeverity::Error,
                field: Some("steps".into()),
                message: "At least one flow step is required.".into(),
            });
        } else {
            if self.steps.len() < rules.min_steps {
                issues.push(FlowSpecIssue {
                    severity: IssueSeverity::Warning,
                    field: Some("steps".into()),
                    message: format!(
                        "Flow has {} steps; recommended minimum is {}.",
                        self.steps.len(),
                        rules.min_steps
                    ),
                });
            }
            if self.steps.len() > rules.max_steps {
                issues.push(FlowSpecIssue {
                    severity: IssueSeverity::Warning,
                    field: Some("steps".into()),
                    message: format!(
                        "Flow has {} steps; recommended maximum is {}.",
                        self.steps.len(),
                        rules.max_steps
                    ),
                });
            }
        }
        if self.invariants.len() < rules.min_invariants {
            issues.push(FlowSpecIssue {
                severity: IssueSeverity::Warning,
                field: Some("invariants".into()),
                message: format!(
                    "Flow has {} invariants; recommended minimum is {}.",
                    self.invariants.len(),
                    rules.min_invariants
                ),
            });
        }
        if self.inputs.is_empty() {
            issues.push(FlowSpecIssue {
                severity: IssueSeverity::Warning,
                field: Some("inputs".into()),
                message: "Inputs list is empty; add key inputs.".into(),
            });
        }
        if self.outputs.is_empty() {
            issues.push(FlowSpecIssue {
                severity: IssueSeverity::Warning,
                field: Some("outputs".into()),
                message: "Outputs list is empty; add key outputs.".into(),
            });
        }
        if self.side_effects.is_empty() {
            issues.push(FlowSpecIssue {
                severity: IssueSeverity::Warning,
                field: Some("side_effects".into()),
                message: "Side effects list is empty; confirm this is pure.".into(),
            });
        }
        if self.failure_modes.is_empty() {
            issues.push(FlowSpecIssue {
                severity: IssueSeverity::Warning,
                field: Some("failure_modes".into()),
                message: "Failure modes list is empty; add at least one.".into(),
            });
        }
        if self.justifications.is_empty() {
            issues.push(FlowSpecIssue {
                severity: IssueSeverity::Warning,
                field: Some("justifications".into()),
                message: "No complexity justifications provided.".into(),
            });
        } else {
            for justification in &self.justifications {
                if justification.item.trim().is_empty() {
                    issues.push(FlowSpecIssue {
                        severity: IssueSeverity::Warning,
                        field: Some("justifications.item".into()),
                        message: "Justification item is empty.".into(),
                    });
                }
                let reason = justification.reason.as_str();
                if !rules
                    .allowed_reasons
                    .iter()
                    .any(|r| r.eq_ignore_ascii_case(reason))
                {
                    issues.push(FlowSpecIssue {
                        severity: IssueSeverity::Warning,
                        field: Some("justifications.reason".into()),
                        message: format!(
                            "Justification reason `{}` is not in allowed list.",
                            reason
                        ),
                    });
                }
            }
        }
        issues
    }
}

pub fn parse_flow_spec(path: &Path, text: &str) -> Result<FlowSpecDocument> {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let (yaml, body) = if matches!(ext.to_lowercase().as_str(), "yml" | "yaml") {
        (text.to_string(), None)
    } else {
        extract_frontmatter(text).with_context(|| {
            format!(
                "Missing or invalid frontmatter in {} (expected leading --- block)",
                path.display()
            )
        })?
    };
    let spec: FlowSpec = serde_yaml::from_str(&yaml)
        .with_context(|| format!("Invalid flow YAML in {}", path.display()))?;
    Ok(FlowSpecDocument {
        path: path.to_path_buf(),
        spec,
        body,
    })
}

fn extract_frontmatter(text: &str) -> Option<(String, Option<String>)> {
    let mut lines = text.lines();
    let first = lines.next()?;
    if first.trim() != "---" {
        return None;
    }
    let mut yaml_lines = Vec::new();
    for line in &mut lines {
        if line.trim() == "---" {
            let rest: String = lines.collect::<Vec<_>>().join("\n");
            let body = if rest.trim().is_empty() {
                None
            } else {
                Some(rest)
            };
            return Some((yaml_lines.join("\n"), body));
        }
        yaml_lines.push(line.to_string());
    }
    None
}
