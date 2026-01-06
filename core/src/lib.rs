//! Deterministic Writing Guard core analysis engine.
//! Implements deterministic rules that flag AI-styled prose based on
//! configurable phrase lists and structural heuristics.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
};

use aho_corasick::{AhoCorasick, AhoCorasickBuilder};
use globset::{Glob, GlobSetBuilder};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

pub mod arch;
pub mod cfg;
pub mod coverage;
pub mod dfg;
pub mod flow;
pub mod symbols;

/// Heading capitalisation policy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum HeadingStyle {
    Any,
    SentenceCase,
    TitleCase,
}

impl Default for HeadingStyle {
    fn default() -> Self {
        HeadingStyle::SentenceCase
    }
}

/// Preferred quotation mark style.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum QuoteStyle {
    Any,
    Straight,
}

impl Default for QuoteStyle {
    fn default() -> Self {
        QuoteStyle::Straight
    }
}

/// Hard limits for stylistic constructs per document section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    pub em_dashes_per_paragraph: usize,
    pub connectors_per_sentence: usize,
    pub rule_of_three_per_paragraph: usize,
    pub bold_spans_per_paragraph: usize,
    pub bold_lead_bullets_per_list: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            em_dashes_per_paragraph: 1,
            connectors_per_sentence: 1,
            rule_of_three_per_paragraph: 0,
            bold_spans_per_paragraph: 3,
            bold_lead_bullets_per_list: 3,
        }
    }
}

/// Thresholds for warnings / failures expressed as flags per 100 words.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ScoreThresholds {
    pub warn_threshold_per_100w: u32,
    pub fail_threshold_per_100w: u32,
}

impl Default for ScoreThresholds {
    fn default() -> Self {
        Self {
            warn_threshold_per_100w: 3,
            fail_threshold_per_100w: 6,
        }
    }
}

/// Whitelisted tokens and phrases that should not trigger diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Whitelist {
    pub allowed_typos: Vec<String>,
    pub allowed_phrases: Vec<String>,
}

impl Default for Whitelist {
    fn default() -> Self {
        Self {
            allowed_typos: vec!["detmerinsitc".into(), "analye".into(), "parallesl".into()],
            allowed_phrases: vec![
                "and then".into(),
                "just ship it".into(),
                "we move on".into(),
            ],
        }
    }
}

/// Phrase container for banned expressions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PhraseList {
    pub ban: Vec<String>,
}

impl Default for PhraseList {
    fn default() -> Self {
        Self { ban: Vec::new() }
    }
}

/// Phrase container for throttled expressions (soft suggestions).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BuzzwordConfig {
    pub throttle: Vec<String>,
}

impl Default for BuzzwordConfig {
    fn default() -> Self {
        Self {
            throttle: vec![
                "delve".into(),
                "delve into".into(),
                "deep dive".into(),
                "leverage".into(),
                "utilise".into(),
                "utilize".into(),
                "facilitate".into(),
                "optimise".into(),
                "optimize".into(),
                "embark".into(),
                "embark on a journey".into(),
                "underscore".into(),
                "aims to explore".into(),
                "aligns".into(),
                "pivotal".into(),
                "vital".into(),
                "robust".into(),
                "innovative".into(),
                "seamless".into(),
                "exemplary".into(),
                "ever-evolving".into(),
                "multifaceted".into(),
                "groundbreaking".into(),
                "holistic".into(),
                "dynamic".into(),
                "paradigm-shifting".into(),
                "landscape".into(),
                "realm".into(),
                "tapestry".into(),
                "efficiency".into(),
                "transformation".into(),
                "synergy".into(),
                "paradigm".into(),
                "roadmap".into(),
                "ecosystem".into(),
                "journey".into(),
                "bandwidth".into(),
                "stakeholder".into(),
                "best practices".into(),
                "strategic implementation".into(),
                "deliverables".into(),
                "adoption rate".into(),
                "capacity building".into(),
                "kpi".into(),
                "proof of concept".into(),
                "cutting-edge".into(),
                "game-changing".into(),
                "next-generation".into(),
                "revolutionary".into(),
                "state-of-the-art".into(),
                "ai-powered".into(),
                "robustly".into(),
                "seamlessly".into(),
                "significantly".into(),
                "notably".into(),
                "fundamentally".into(),
                "inherently".into(),
                "transformative".into(),
                "journey of".into(),
                "unprecedented".into(),
                "plethora".into(),
                "empower".into(),
                "comprehensive".into(),
                "streamlined".into(),
                "scalable".into(),
                "actionable insights".into(),
                "data-driven".into(),
            ],
        }
    }
}

/// Profile-specific rule overrides applied to matched files.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileRules {
    pub max_headings: Option<usize>,
    pub required_headings: Vec<String>,
    pub banned_headings: Vec<String>,
    pub call_to_action_phrases: Vec<String>,
    pub template_phrases: Vec<String>,
    pub max_sentence_length: Option<usize>,
    pub max_duplicate_sentences: Option<usize>,
    pub cadence_starts: Vec<String>,
    pub cadence_limit: Option<usize>,
    pub broad_terms: Vec<String>,
    pub confidence_phrases: Vec<String>,
    pub max_heading_depth: Option<usize>,
    pub max_bullet_items: Option<usize>,
    pub forbid_rhetorical_headings: bool,
    pub required_patterns: Vec<String>,
    pub forbidden_patterns: Vec<String>,
    pub max_exclamations_per_paragraph: Option<usize>,
    pub question_lead_limit: Option<usize>,
    pub min_sentences_per_section: Option<usize>,
    pub min_code_blocks: Option<usize>,
    pub enable_triad_slop: bool,
}

impl Default for ProfileRules {
    fn default() -> Self {
        Self {
            max_headings: None,
            required_headings: Vec::new(),
            banned_headings: Vec::new(),
            call_to_action_phrases: Vec::new(),
            template_phrases: Vec::new(),
            max_sentence_length: None,
            max_duplicate_sentences: None,
            cadence_starts: Vec::new(),
            cadence_limit: None,
            broad_terms: Vec::new(),
            confidence_phrases: Vec::new(),
            max_heading_depth: Some(3),
            max_bullet_items: Some(7),
            forbid_rhetorical_headings: true,
            required_patterns: Vec::new(),
            forbidden_patterns: Vec::new(),
            max_exclamations_per_paragraph: Some(1),
            question_lead_limit: Some(1),
            min_sentences_per_section: None,
            min_code_blocks: None,
            enable_triad_slop: true,
        }
    }
}

/// File matching configuration to attach rule overrides.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProfileConfig {
    pub name: String,
    pub globs: Vec<String>,
    pub extends: Option<String>,
    pub rules: ProfileRules,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CommentPolicy {
    pub enabled: bool,
    pub max_ratio: Option<f32>,
    pub ignore_globs: Vec<String>,
    pub allow_globs: Vec<String>,
    pub keywords: Vec<String>,
    pub ticket_reference_regex: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RepoRules {
    pub ignore_globs: Vec<String>,
    pub slop_globs: Vec<String>,
    pub banned_dirs: Vec<String>,
    pub suspicious_filenames: Vec<String>,
    pub large_json_globs: Vec<String>,
    pub allow_large_json_globs: Vec<String>,
    pub large_json_limit_kb: Option<u64>,
    pub duplicate_lock_check: bool,
}

impl Default for RepoRules {
    fn default() -> Self {
        Self {
            ignore_globs: vec![
                "vendor/**".into(),
                "third_party/**".into(),
                "node_modules/**".into(),
                "**/*.min.*".into(),
                "dist/**".into(),
                "build/**".into(),
                "**/.git/**".into(),
                "target/**".into(),
                "**/node_modules/**".into(),
                "**/dist/**".into(),
                "**/build/**".into(),
                "**/target/**".into(),
                "**/out/**".into(),
                "**/coverage/**".into(),
                "**/.idea/**".into(),
            ],
            slop_globs: vec![
                "**/*copy*".into(),
                "**/*backup*".into(),
                "**/*old*".into(),
                "**/*new*".into(),
                "**/*final*".into(),
                "**/*(1)*".into(),
                "**/*(2)*".into(),
                "**/*-draft*".into(),
            ],
            banned_dirs: vec![
                "__pycache__".into(),
                ".pytest_cache".into(),
                ".idea".into(),
                ".vscode".into(),
                ".DS_Store".into(),
                "Thumbs.db".into(),
            ],
            suspicious_filenames: vec![
                "(?i)(copy|backup|old|new|final(_?final)?|final2|\\(\\d+\\)|-draft|cleanup|helper|utils2|script_final)".into(),
            ],
            large_json_globs: vec!["**/*.json".into(), "**/*.yaml".into(), "**/*.yml".into()],
            allow_large_json_globs: vec!["fixtures/**".into(), "tests/fixtures/**".into(), "data/raw/**".into()],
            large_json_limit_kb: Some(500),
            duplicate_lock_check: true,
        }
    }
}

impl Default for CommentPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            max_ratio: Some(0.05),
            ignore_globs: Vec::new(),
            allow_globs: Vec::new(),
            keywords: vec![
                "TODO".into(),
                "FIXME".into(),
                "HACK".into(),
                "XXX".into(),
                "KLUDGE".into(),
                "TEMP".into(),
                "WORKAROUND".into(),
                "BUG".into(),
                "WIP".into(),
                "OPTIMIZE".into(),
                "CHEAT".into(),
                "DIRTY".into(),
                "QUICK AND DIRTY".into(),
            ],
            ticket_reference_regex: Some("(?i)(ticket|issue|jira|#\\d+)".into()),
        }
    }
}

/// Top-level configuration for the analyzer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub heading_style: HeadingStyle,
    pub quote_style: QuoteStyle,
    pub limits: Limits,
    pub scores: ScoreThresholds,
    pub whitelist: Whitelist,
    pub buzzwords: BuzzwordConfig,
    pub transitions: BuzzwordConfig,
    pub puffery: PhraseList,
    pub templates: PhraseList,
    pub weasel: PhraseList,
    pub marketing_cliches: PhraseList,
    pub profile_defaults: ProfileRules,
    pub profiles: Vec<ProfileConfig>,
    pub repo_rules: RepoRules,
    pub comment_policy: CommentPolicy,
    pub flow_rules: flow::FlowRules,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            heading_style: HeadingStyle::SentenceCase,
            quote_style: QuoteStyle::Straight,
            limits: Limits::default(),
            scores: ScoreThresholds::default(),
            whitelist: Whitelist::default(),
            buzzwords: BuzzwordConfig::default(),
            transitions: BuzzwordConfig {
                throttle: vec![
                    "furthermore".into(),
                    "moreover".into(),
                    "consequently".into(),
                    "thus".into(),
                    "accordingly".into(),
                    "nonetheless".into(),
                    "subsequently".into(),
                    "therefore".into(),
                    "at the same time".into(),
                    "to that end".into(),
                    "in addition to".into(),
                    "alongside this".into(),
                    "as a result".into(),
                    "in fact".into(),
                    "in essence".into(),
                    "in summary".into(),
                    "significantly".into(),
                    "remarkably".into(),
                    "notably".into(),
                    "additionally".into(),
                    "in other words".into(),
                    "on the other hand".into(),
                    "in contrast".into(),
                    "to summarize".into(),
                ],
            },
            puffery: PhraseList {
                ban: vec![
                    "rich cultural heritage".into(),
                    "vibrant cultural heritage".into(),
                    "cultural tapestry".into(),
                    "breathtaking".into(),
                    "must-visit".into(),
                    "must-see".into(),
                    "stunning natural beauty".into(),
                    "enduring legacy".into(),
                    "lasting legacy".into(),
                    "nestled".into(),
                    "in the heart of".into(),
                    "stands as a symbol of".into(),
                    "stands as a testament".into(),
                    "plays a pivotal role in".into(),
                    "leaves a lasting impact".into(),
                    "hallmark of innovation".into(),
                    "gateway to".into(),
                    "thriving ecosystem".into(),
                    "vibrant ecosystem".into(),
                    "groundbreaking innovation".into(),
                    "unparalleled excellence".into(),
                    "a seamless journey".into(),
                    "a diverse tapestry".into(),
                    "ultimate solution".into(),
                    "one-stop shop".into(),
                    "all-in-one platform".into(),
                    "future-proof".into(),
                    "game-changing".into(),
                    "trusted by leading brands".into(),
                ],
            },
            templates: PhraseList {
                ban: vec![
                    "^in conclusion".into(),
                    "^overall".into(),
                    "^in summary".into(),
                    "^in essence".into(),
                    "^future prospects include".into(),
                    "^in today’s fast-paced world".into(),
                    "^in today's fast-paced world".into(),
                    "^in today’s ever-evolving world".into(),
                    "^in today's ever-evolving world".into(),
                    "\\bit is worth noting\\b".into(),
                    "\\bit is important to note\\b".into(),
                    "\\bit should be mentioned\\b".into(),
                    "\\bit is worth considering\\b".into(),
                    "\\bone might argue\\b".into(),
                    "\\bone could contend\\b".into(),
                    "\\bbased on the information provided\\b".into(),
                    "\\baccording to the data\\b".into(),
                    "\\bevidently, this suggests\\b".into(),
                    "\\bnot (?:just|only)\\b.+\\bbut (?:also|rather)\\b".into(),
                    "\\bno [^,.;]+, no [^,.;]+, just [^,.;]+".into(),
                    "\\bplay(?:s)? a significant role in shaping\\b".into(),
                    "\\baims to explore\\b".into(),
                    "\\btoday’s fast-paced world\\b".into(),
                    "\\btoday's fast-paced world\\b".into(),
                    "(?m)^subject:".into(),
                    "(?m)^re:".into(),
                    "\\bas an ai language model\\b".into(),
                    "\\bas a language model\\b".into(),
                    "\\bmy (?:knowledge|training) (?:cutoff|cut-off)\\b".into(),
                    "\\bi (?:do not|don't) have access\\b".into(),
                    "\\bi (?:do not|don't) have the ability\\b".into(),
                    "\\bi cannot access\\b".into(),
                    "\\bi hope this helps\\b".into(),
                    "\\blet me know if you have any questions\\b".into(),
                    "\\bfeel free to reach out\\b".into(),
                    "\\bin this (?:article|post)\\b".into(),
                    "\\bthis (?:article|post) (?:covers|explores|will cover)\\b".into(),
                    "\\blet's dive in\\b".into(),
                    "\\bas technology continues to evolve\\b".into(),
                    "\\bin an? ever-evolving (?:world|landscape|industry)\\b".into(),
                    "\\bin the modern (?:world|era)\\b".into(),
                    "\\bat the end of the day\\b".into(),
                    "\\bthe following (?:section|sections) (?:covers|cover|will cover)\\b".into(),
                    "\\bchallenges and opportunities\\b".into(),
                    "\\bfuture (?:outlook|directions|prospects)\\b".into(),
                    "\\bdespite these challenges\\b".into(),
                    "\\blooking ahead\\b".into(),
                    "\\bkey takeaways?\\b".into(),
                ],
            },
            weasel: PhraseList {
                ban: vec![
                    "some critics argue".into(),
                    "experts say".into(),
                    "observers noted".into(),
                    "industry reports show".into(),
                    "it should be mentioned that".into(),
                    "it is worth considering that".into(),
                    "it could be suggested that".into(),
                    "many experts believe".into(),
                    "it is widely believed".into(),
                    "it is often said".into(),
                    "some would argue".into(),
                    "various sources suggest".into(),
                    "some sources say".into(),
                    "research suggests".into(),
                    "studies show".into(),
                ],
            },
            marketing_cliches: PhraseList {
                ban: vec![
                    "unlock the power of".into(),
                    "revolutionise the way".into(),
                    "revolutionize the way".into(),
                    "take your business to the next level".into(),
                    "game-changing solution".into(),
                    "unparalleled excellence".into(),
                    "cutting-edge technology".into(),
                    "seamlessly integrated".into(),
                    "state-of-the-art".into(),
                    "disruptive innovation".into(),
                    "next-generation".into(),
                    "seamless experience".into(),
                    "delightful experience".into(),
                    "unlock your potential".into(),
                    "empower your".into(),
                    "limited time offer".into(),
                    "don’t miss out".into(),
                    "don't miss out".into(),
                    "act now".into(),
                ],
            },
            profile_defaults: ProfileRules {
                max_headings: None,
                required_headings: Vec::new(),
                banned_headings: Vec::new(),
                call_to_action_phrases: vec![
                    "start your free trial".into(),
                    "try it free".into(),
                    "get started for free".into(),
                    "download today".into(),
                    "get started now".into(),
                    "join thousands".into(),
                    "join the revolution".into(),
                    "book a demo".into(),
                    "contact sales".into(),
                    "subscribe now".into(),
                    "join the waitlist".into(),
                    "reserve your spot".into(),
                    "apply now".into(),
                    "get early access".into(),
                    "limited seats".into(),
                    "act now".into(),
                    "unlock access".into(),
                    "claim your offer".into(),
                ],
                template_phrases: Vec::new(),
                max_sentence_length: Some(28),
                max_duplicate_sentences: Some(1),
                cadence_starts: vec!["we".into(), "our".into(), "toneguard".into()],
                cadence_limit: Some(2),
                broad_terms: vec![
                    "solution".into(),
                    "platform".into(),
                    "ecosystem".into(),
                    "experience".into(),
                    "vision".into(),
                    "mission".into(),
                    "innovation".into(),
                    "framework".into(),
                    "learnings".into(),
                    "journey".into(),
                ],
                confidence_phrases: vec![
                    "industry-leading".into(),
                    "world-class".into(),
                    "unrivaled".into(),
                    "leading provider".into(),
                    "best-in-class".into(),
                    "top-tier".into(),
                    "number one".into(),
                    "unmatched".into(),
                    "trusted by leading brands".into(),
                ],
                max_heading_depth: Some(3),
                max_bullet_items: Some(7),
                forbid_rhetorical_headings: true,
                required_patterns: Vec::new(),
                forbidden_patterns: Vec::new(),
                max_exclamations_per_paragraph: Some(1),
                question_lead_limit: Some(1),
                min_sentences_per_section: None,
                min_code_blocks: None,
                enable_triad_slop: true,
            },
            profiles: Vec::new(),
            repo_rules: RepoRules::default(),
            comment_policy: CommentPolicy::default(),
            flow_rules: flow::FlowRules::default(),
        }
    }
}

#[derive(Debug, Clone)]
struct ProfileRecipe {
    name: String,
    max_headings: Option<usize>,
    required_headings: Vec<String>,
    banned_headings: Vec<String>,
    call_to_action_phrases: Vec<String>,
    template_phrases: Vec<String>,
    max_sentence_length: Option<usize>,
    max_duplicate_sentences: Option<usize>,
    cadence_starts: Vec<String>,
    cadence_limit: Option<usize>,
    broad_terms: Vec<String>,
    confidence_phrases: Vec<String>,
    max_heading_depth: Option<usize>,
    max_bullet_items: Option<usize>,
    forbid_rhetorical_headings: bool,
    required_patterns: Vec<String>,
    forbidden_patterns: Vec<String>,
    max_exclamations_per_paragraph: Option<usize>,
    question_lead_limit: Option<usize>,
    min_sentences_per_section: Option<usize>,
    min_code_blocks: Option<usize>,
    enable_triad_slop: bool,
}

impl ProfileRecipe {
    fn from_rules(name: impl Into<String>, base: &ProfileRules) -> Self {
        Self {
            name: name.into(),
            max_headings: base.max_headings,
            required_headings: base
                .required_headings
                .iter()
                .map(|s| s.to_lowercase())
                .collect(),
            banned_headings: base.banned_headings.iter().map(|s| s.to_string()).collect(),
            call_to_action_phrases: base.call_to_action_phrases.clone(),
            template_phrases: base.template_phrases.clone(),
            max_sentence_length: base.max_sentence_length,
            max_duplicate_sentences: base.max_duplicate_sentences,
            cadence_starts: base
                .cadence_starts
                .iter()
                .map(|s| s.to_lowercase())
                .collect(),
            cadence_limit: base.cadence_limit,
            broad_terms: base.broad_terms.iter().map(|s| s.to_lowercase()).collect(),
            confidence_phrases: base.confidence_phrases.clone(),
            max_heading_depth: base.max_heading_depth,
            max_bullet_items: base.max_bullet_items,
            forbid_rhetorical_headings: base.forbid_rhetorical_headings,
            required_patterns: base.required_patterns.clone(),
            forbidden_patterns: base.forbidden_patterns.clone(),
            max_exclamations_per_paragraph: base.max_exclamations_per_paragraph,
            question_lead_limit: base.question_lead_limit,
            min_sentences_per_section: base.min_sentences_per_section,
            min_code_blocks: base.min_code_blocks,
            enable_triad_slop: base.enable_triad_slop,
        }
    }

    fn extend_with(&mut self, overrides: &ProfileRules) {
        if overrides.max_headings.is_some() {
            self.max_headings = overrides.max_headings;
        }
        if overrides.max_sentence_length.is_some() {
            self.max_sentence_length = overrides.max_sentence_length;
        }
        if overrides.max_duplicate_sentences.is_some() {
            self.max_duplicate_sentences = overrides.max_duplicate_sentences;
        }
        if !overrides.required_headings.is_empty() {
            for heading in &overrides.required_headings {
                self.required_headings.push(heading.to_lowercase());
            }
        }
        if !overrides.banned_headings.is_empty() {
            for heading in &overrides.banned_headings {
                self.banned_headings.push(heading.clone());
            }
        }
        if !overrides.call_to_action_phrases.is_empty() {
            for phrase in &overrides.call_to_action_phrases {
                self.call_to_action_phrases.push(phrase.clone());
            }
        }
        if !overrides.template_phrases.is_empty() {
            for phrase in &overrides.template_phrases {
                self.template_phrases.push(phrase.clone());
            }
        }
        if !overrides.cadence_starts.is_empty() {
            for start in &overrides.cadence_starts {
                self.cadence_starts.push(start.to_lowercase());
            }
        }
        if overrides.cadence_limit.is_some() {
            self.cadence_limit = overrides.cadence_limit;
        }
        if !overrides.broad_terms.is_empty() {
            for term in &overrides.broad_terms {
                self.broad_terms.push(term.to_lowercase());
            }
        }
        if !overrides.confidence_phrases.is_empty() {
            for phrase in &overrides.confidence_phrases {
                self.confidence_phrases.push(phrase.clone());
            }
        }
        if overrides.max_heading_depth.is_some() {
            self.max_heading_depth = overrides.max_heading_depth;
        }
        if overrides.max_bullet_items.is_some() {
            self.max_bullet_items = overrides.max_bullet_items;
        }
        if overrides.forbid_rhetorical_headings {
            self.forbid_rhetorical_headings = true;
        }
        if !overrides.required_patterns.is_empty() {
            for pattern in &overrides.required_patterns {
                self.required_patterns.push(pattern.clone());
            }
        }
        if !overrides.forbidden_patterns.is_empty() {
            for pattern in &overrides.forbidden_patterns {
                self.forbidden_patterns.push(pattern.clone());
            }
        }
        if overrides.max_exclamations_per_paragraph.is_some() {
            self.max_exclamations_per_paragraph = overrides.max_exclamations_per_paragraph;
        }
        if overrides.question_lead_limit.is_some() {
            self.question_lead_limit = overrides.question_lead_limit;
        }
        if overrides.min_sentences_per_section.is_some() {
            self.min_sentences_per_section = overrides.min_sentences_per_section;
        }
        if overrides.min_code_blocks.is_some() {
            self.min_code_blocks = overrides.min_code_blocks;
        }
        if overrides.enable_triad_slop {
            self.enable_triad_slop = true;
        }
    }

    fn clone_for(&self, name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            max_headings: self.max_headings,
            required_headings: self.required_headings.clone(),
            banned_headings: self.banned_headings.clone(),
            call_to_action_phrases: self.call_to_action_phrases.clone(),
            template_phrases: self.template_phrases.clone(),
            max_sentence_length: self.max_sentence_length,
            max_duplicate_sentences: self.max_duplicate_sentences,
            cadence_starts: self.cadence_starts.clone(),
            cadence_limit: self.cadence_limit,
            broad_terms: self.broad_terms.clone(),
            confidence_phrases: self.confidence_phrases.clone(),
            max_heading_depth: self.max_heading_depth,
            max_bullet_items: self.max_bullet_items,
            forbid_rhetorical_headings: self.forbid_rhetorical_headings,
            required_patterns: self.required_patterns.clone(),
            forbidden_patterns: self.forbidden_patterns.clone(),
            max_exclamations_per_paragraph: self.max_exclamations_per_paragraph,
            question_lead_limit: self.question_lead_limit,
            min_sentences_per_section: self.min_sentences_per_section,
            min_code_blocks: self.min_code_blocks,
            enable_triad_slop: self.enable_triad_slop,
        }
    }
}

#[derive(Debug, Clone)]
struct ProfileRuntime {
    name: String,
    max_headings: Option<usize>,
    required_headings: Vec<String>,
    banned_heading_regexes: Vec<Regex>,
    call_to_action_matcher: Option<AhoCorasick>,
    template_regexes: Vec<Regex>,
    max_sentence_length: Option<usize>,
    max_duplicate_sentences: usize,
    cadence_starts: Vec<String>,
    cadence_limit: usize,
    broad_terms: Vec<String>,
    confidence_matcher: Option<AhoCorasick>,
    detect_percent_claims: bool,
    max_heading_depth: Option<usize>,
    max_bullet_items: Option<usize>,
    forbid_rhetorical_headings: bool,
    required_patterns: Vec<Regex>,
    forbidden_patterns: Vec<Regex>,
    max_exclamations_per_paragraph: Option<usize>,
    question_lead_limit: Option<usize>,
    min_sentences_per_section: Option<usize>,
    min_code_blocks: Option<usize>,
    enable_triad_slop: bool,
}

impl ProfileRuntime {
    fn compile(recipe: ProfileRecipe) -> anyhow::Result<Self> {
        let mut banned_heading_regexes = Vec::new();
        for pattern in &recipe.banned_headings {
            let regex = Regex::new(&format!("(?i){}", pattern))?;
            banned_heading_regexes.push(regex);
        }
        let mut template_regexes = Vec::new();
        for pattern in &recipe.template_phrases {
            let regex = Regex::new(&format!("(?i){}", pattern))?;
            template_regexes.push(regex);
        }
        let mut required_patterns = Vec::new();
        for pattern in &recipe.required_patterns {
            let regex = Regex::new(&format!("(?i){}", pattern))?;
            required_patterns.push(regex);
        }
        let mut forbidden_patterns = Vec::new();
        for pattern in &recipe.forbidden_patterns {
            let regex = Regex::new(&format!("(?i){}", pattern))?;
            forbidden_patterns.push(regex);
        }
        let call_to_action_matcher = if recipe.call_to_action_phrases.is_empty() {
            None
        } else {
            Some(
                AhoCorasickBuilder::new()
                    .ascii_case_insensitive(true)
                    .build(recipe.call_to_action_phrases.clone()),
            )
        };
        let confidence_matcher = if recipe.confidence_phrases.is_empty() {
            None
        } else {
            Some(
                AhoCorasickBuilder::new()
                    .ascii_case_insensitive(true)
                    .build(recipe.confidence_phrases.clone()),
            )
        };
        Ok(Self {
            name: recipe.name,
            max_headings: recipe.max_headings,
            required_headings: recipe.required_headings,
            banned_heading_regexes,
            call_to_action_matcher,
            template_regexes,
            max_sentence_length: recipe.max_sentence_length,
            max_duplicate_sentences: recipe.max_duplicate_sentences.unwrap_or(1),
            cadence_starts: recipe.cadence_starts,
            cadence_limit: recipe.cadence_limit.unwrap_or(2).max(1),
            broad_terms: recipe.broad_terms,
            confidence_matcher,
            detect_percent_claims: true,
            max_heading_depth: recipe.max_heading_depth,
            max_bullet_items: recipe.max_bullet_items,
            forbid_rhetorical_headings: recipe.forbid_rhetorical_headings,
            required_patterns,
            forbidden_patterns,
            max_exclamations_per_paragraph: recipe.max_exclamations_per_paragraph,
            question_lead_limit: recipe.question_lead_limit,
            min_sentences_per_section: recipe.min_sentences_per_section,
            min_code_blocks: recipe.min_code_blocks,
            enable_triad_slop: recipe.enable_triad_slop,
        })
    }
}

struct ProfileMatcher {
    name: String,
    globs: globset::GlobSet,
}

#[derive(Clone)]
struct HeadingCapture {
    line: usize,
    column: usize,
    offset: usize,
    len: usize,
    text: String,
    lower: String,
}

#[derive(Clone, Debug)]
struct PhraseHit {
    start: usize,
    end: usize,
    snippet: String,
    suggestion: Option<String>,
    sentence_idx: usize,
}

fn resolve_profile_recipe(
    name: &str,
    configs: &HashMap<String, ProfileConfig>,
    cache: &mut HashMap<String, ProfileRecipe>,
    defaults: &ProfileRules,
) -> anyhow::Result<ProfileRecipe> {
    if let Some(recipe) = cache.get(name) {
        return Ok(recipe.clone());
    }

    if name == "default" {
        let recipe = ProfileRecipe::from_rules(name.to_string(), defaults);
        cache.insert(name.to_string(), recipe.clone());
        return Ok(recipe);
    }

    let config = configs
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("unknown profile `{}`", name))?;

    let base_recipe = if let Some(parent) = &config.extends {
        let parent_recipe = resolve_profile_recipe(parent, configs, cache, defaults)?;
        parent_recipe.clone_for(name)
    } else {
        ProfileRecipe::from_rules(name.to_string(), defaults)
    };

    let mut recipe = base_recipe;
    recipe.extend_with(&config.rules);
    cache.insert(name.to_string(), recipe.clone());
    Ok(recipe)
}

/// Rule category identifiers.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Ord, PartialOrd)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    Puffery,
    Buzzword,
    NegativeParallel,
    RuleOfThree,
    ConnectorGlut,
    Template,
    Weasel,
    Transition,
    Marketing,
    Structure,
    CallToAction,
    SentenceLength,
    Repetition,
    Cadence,
    Confidence,
    BroadTerm,
    Tone,
    EmDash,
    Formatting,
    QuoteStyle,
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Category::Puffery => "puffery",
            Category::Buzzword => "buzzword",
            Category::NegativeParallel => "negative-parallelism",
            Category::RuleOfThree => "rule-of-three",
            Category::ConnectorGlut => "connector-glut",
            Category::Template => "template",
            Category::Weasel => "weasel",
            Category::Transition => "transition",
            Category::Marketing => "marketing",
            Category::Structure => "structure",
            Category::CallToAction => "call-to-action",
            Category::SentenceLength => "sentence-length",
            Category::Repetition => "repetition",
            Category::Cadence => "cadence",
            Category::Confidence => "confidence",
            Category::BroadTerm => "broad-term",
            Category::Tone => "tone",
            Category::EmDash => "em-dash",
            Category::Formatting => "formatting",
            Category::QuoteStyle => "quote-style",
        };
        f.write_str(name)
    }
}

pub fn parse_category(name: &str) -> Option<Category> {
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

/// Location metadata in 1-based line/column coordinates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub line: usize,
    pub column: usize,
}

/// Diagnostic severity levels matching LSP specification.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Ord, PartialOrd)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    /// Hard slop signal - always flag
    Error,
    /// Likely slop - clustered patterns or strong signals
    Warning,
    /// Style suggestion - single soft signals
    Hint,
    /// Informational only
    Information,
}

impl Default for Severity {
    fn default() -> Self {
        Severity::Warning
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Error => f.write_str("error"),
            Severity::Warning => f.write_str("warning"),
            Severity::Hint => f.write_str("hint"),
            Severity::Information => f.write_str("info"),
        }
    }
}

/// Style diagnostic emitted by the analyzer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub category: Category,
    pub severity: Severity,
    pub message: String,
    pub suggestion: Option<String>,
    pub location: Location,
    pub span: (usize, usize),
    pub snippet: String,
}

/// Summary statistics for a document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentReport {
    pub word_count: usize,
    pub diagnostics: Vec<Diagnostic>,
    pub category_counts: BTreeMap<Category, usize>,
    pub profile: String,
}

impl DocumentReport {
    /// Style density = flags per 100 words (rounded up).
    pub fn density_per_100_words(&self) -> f32 {
        if self.word_count == 0 {
            return self.diagnostics.len() as f32;
        }
        (self.diagnostics.len() as f32) * 100.0 / (self.word_count as f32)
    }
}

/// Analyzer encapsulates compiled rules for reuse across files.
pub struct Analyzer {
    config: Config,
    allow_phrase_set: HashSet<String>,
    puffery_matcher: Option<AhoCorasick>,
    buzzword_matcher: Option<AhoCorasick>,
    weasel_matcher: Option<AhoCorasick>,
    transition_matcher: Option<AhoCorasick>,
    marketing_matcher: Option<AhoCorasick>,
    base_template_regexes: Vec<Regex>,
    rule_of_three_regex: Regex,
    range_regex: Regex,
    profile_runtimes: HashMap<String, ProfileRuntime>,
    profile_matchers: Vec<ProfileMatcher>,
    default_profile: String,
}

impl Analyzer {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let allow_phrase_set = config
            .whitelist
            .allowed_phrases
            .iter()
            .map(|s| s.to_lowercase())
            .collect();

        let puffery_matcher = if config.puffery.ban.is_empty() {
            None
        } else {
            Some(
                AhoCorasickBuilder::new()
                    .ascii_case_insensitive(true)
                    .build(&config.puffery.ban),
            )
        };

        let buzzword_matcher = if config.buzzwords.throttle.is_empty() {
            None
        } else {
            Some(
                AhoCorasickBuilder::new()
                    .ascii_case_insensitive(true)
                    .build(&config.buzzwords.throttle),
            )
        };

        let weasel_matcher = if config.weasel.ban.is_empty() {
            None
        } else {
            Some(
                AhoCorasickBuilder::new()
                    .ascii_case_insensitive(true)
                    .build(&config.weasel.ban),
            )
        };

        let transition_matcher = if config.transitions.throttle.is_empty() {
            None
        } else {
            Some(
                AhoCorasickBuilder::new()
                    .ascii_case_insensitive(true)
                    .build(&config.transitions.throttle),
            )
        };

        let marketing_matcher = if config.marketing_cliches.ban.is_empty() {
            None
        } else {
            Some(
                AhoCorasickBuilder::new()
                    .ascii_case_insensitive(true)
                    .build(&config.marketing_cliches.ban),
            )
        };

        let mut base_template_regexes = Vec::new();
        for pattern in &config.templates.ban {
            let pattern = pattern.trim();
            if pattern.is_empty() {
                continue;
            }
            let regex = Regex::new(&format!("(?i){pattern}"))
                .map_err(|e| anyhow::anyhow!("invalid template regex `{pattern}`: {e}"))?;
            base_template_regexes.push(regex);
        }

        let rule_of_three_regex =
            Regex::new(r"(?i)\b[\w-]+,\s+[\w-]+,\s+(?:and|&)\s+[\w-]+").expect("static regex");

        let range_regex =
            Regex::new(r"(?i)from [^\n,.;]+ to [^\n,.;]+ to [^\n,.;]+").expect("static regex");

        let mut profile_config_map: HashMap<String, ProfileConfig> = HashMap::new();
        for profile in &config.profiles {
            if profile.name.trim().is_empty() {
                continue;
            }
            profile_config_map.insert(profile.name.clone(), profile.clone());
        }

        let mut recipe_cache: HashMap<String, ProfileRecipe> = HashMap::new();
        let default_recipe = resolve_profile_recipe(
            "default",
            &profile_config_map,
            &mut recipe_cache,
            &config.profile_defaults,
        )?;
        let default_runtime = ProfileRuntime::compile(default_recipe.clone())?;
        let mut profile_runtimes: HashMap<String, ProfileRuntime> = HashMap::new();
        profile_runtimes.insert(default_runtime.name.clone(), default_runtime);

        let profile_names: Vec<String> = profile_config_map.keys().cloned().collect();
        for name in profile_names {
            let recipe = resolve_profile_recipe(
                &name,
                &profile_config_map,
                &mut recipe_cache,
                &config.profile_defaults,
            )?;
            let runtime = ProfileRuntime::compile(recipe)?;
            profile_runtimes.insert(runtime.name.clone(), runtime);
        }

        let mut profile_matchers = Vec::new();
        for profile in &config.profiles {
            if profile.name.trim().is_empty() || profile.globs.is_empty() {
                continue;
            }
            let mut builder = GlobSetBuilder::new();
            for pattern in &profile.globs {
                let glob = Glob::new(pattern).map_err(|e| {
                    anyhow::anyhow!(
                        "invalid glob `{pattern}` in profile `{}`: {e}",
                        profile.name
                    )
                })?;
                builder.add(glob);
            }
            let globs = builder.build().map_err(|e| {
                anyhow::anyhow!(
                    "failed to build globset for profile `{}`: {e}",
                    profile.name
                )
            })?;
            profile_matchers.push(ProfileMatcher {
                name: profile.name.clone(),
                globs,
            });
        }

        Ok(Self {
            config,
            allow_phrase_set,
            puffery_matcher,
            buzzword_matcher,
            weasel_matcher,
            transition_matcher,
            marketing_matcher,
            base_template_regexes,
            rule_of_three_regex,
            range_regex,
            profile_runtimes,
            profile_matchers,
            default_profile: "default".into(),
        })
    }

    pub fn default_profile(&self) -> &str {
        &self.default_profile
    }

    fn profile_for_name(&self, name: &str) -> Option<&ProfileRuntime> {
        self.profile_runtimes.get(name)
    }

    pub fn profile_for_path(&self, relative_path: &str) -> &str {
        let path = Path::new(relative_path);
        for matcher in &self.profile_matchers {
            if matcher.globs.is_match(path) {
                if self.profile_runtimes.contains_key(&matcher.name) {
                    return &matcher.name;
                }
            }
        }
        &self.default_profile
    }

    pub fn analyze_profile_name(
        &self,
        text: &str,
        profile_name: &str,
    ) -> anyhow::Result<DocumentReport> {
        let profile = self
            .profile_for_name(profile_name)
            .ok_or_else(|| anyhow::anyhow!("unknown profile `{profile_name}`"))?;
        Ok(self.analyze_with_profile(text, profile))
    }

    pub fn analyze(&self, text: &str) -> DocumentReport {
        let profile = self
            .profile_runtimes
            .get(&self.default_profile)
            .expect("default profile missing");
        self.analyze_with_profile(text, profile)
    }

    pub(crate) fn analyze_with_profile(
        &self,
        text: &str,
        profile: &ProfileRuntime,
    ) -> DocumentReport {
        let filtered = DisabledRanges::new(text);
        let mut diagnostics = Vec::new();
        let mut category_counts: BTreeMap<Category, usize> = BTreeMap::new();
        let sentences = split_sentences_with_offset(text);

        self.detect_puffery(
            text,
            &sentences,
            &filtered,
            &mut diagnostics,
            &mut category_counts,
        );
        self.detect_buzzwords(
            text,
            &sentences,
            &filtered,
            &mut diagnostics,
            &mut category_counts,
        );
        self.detect_transitions(
            text,
            &sentences,
            &filtered,
            &mut diagnostics,
            &mut category_counts,
        );
        self.detect_marketing(text, &filtered, &mut diagnostics, &mut category_counts);
        self.detect_templates(
            text,
            &filtered,
            profile,
            &mut diagnostics,
            &mut category_counts,
        );
        self.detect_ranges(text, &filtered, &mut diagnostics, &mut category_counts);

        self.detect_connectors(
            text,
            &sentences,
            &filtered,
            &mut diagnostics,
            &mut category_counts,
        );
        self.detect_sentence_length(
            text,
            &sentences,
            &filtered,
            profile,
            &mut diagnostics,
            &mut category_counts,
        );
        self.detect_question_lead(
            text,
            &sentences,
            &filtered,
            profile,
            &mut diagnostics,
            &mut category_counts,
        );
        self.detect_mid_sentence_questions(
            text,
            &sentences,
            &filtered,
            &mut diagnostics,
            &mut category_counts,
        );
        self.detect_cadence(
            text,
            &sentences,
            &filtered,
            profile,
            &mut diagnostics,
            &mut category_counts,
        );
        self.detect_broad_terms(
            text,
            &sentences,
            &filtered,
            profile,
            &mut diagnostics,
            &mut category_counts,
        );
        self.detect_repetition(
            text,
            &sentences,
            &filtered,
            profile,
            &mut diagnostics,
            &mut category_counts,
        );
        self.detect_exclamation_density(
            text,
            &filtered,
            profile,
            &mut diagnostics,
            &mut category_counts,
        );

        self.detect_rule_of_three(text, &filtered, &mut diagnostics, &mut category_counts);
        self.detect_em_dash(text, &filtered, &mut diagnostics, &mut category_counts);
        self.detect_bold_spans(text, &filtered, &mut diagnostics, &mut category_counts);
        self.detect_headings(
            text,
            &filtered,
            profile,
            &mut diagnostics,
            &mut category_counts,
        );
        self.detect_bullet_items(
            text,
            &filtered,
            profile,
            &mut diagnostics,
            &mut category_counts,
        );
        self.detect_emoji_bullets(text, &filtered, &mut diagnostics, &mut category_counts);
        self.detect_bold_lead_bullets(text, &filtered, &mut diagnostics, &mut category_counts);
        self.detect_call_to_action(
            text,
            &filtered,
            profile,
            &mut diagnostics,
            &mut category_counts,
        );
        self.detect_confidence(
            text,
            &sentences,
            &filtered,
            profile,
            &mut diagnostics,
            &mut category_counts,
        );
        self.detect_section_density(
            text,
            &sentences,
            &filtered,
            profile,
            &mut diagnostics,
            &mut category_counts,
        );
        self.detect_triad_slop(
            text,
            &filtered,
            profile,
            &mut diagnostics,
            &mut category_counts,
        );
        self.detect_min_code_blocks(
            text,
            &filtered,
            profile,
            &mut diagnostics,
            &mut category_counts,
        );
        self.detect_required_patterns(
            text,
            &filtered,
            profile,
            &mut diagnostics,
            &mut category_counts,
        );
        self.detect_forbidden_patterns(
            text,
            &filtered,
            profile,
            &mut diagnostics,
            &mut category_counts,
        );
        self.detect_quotes(text, &filtered, &mut diagnostics, &mut category_counts);
        self.detect_statistical_slop(
            text,
            &sentences,
            &filtered,
            &mut diagnostics,
            &mut category_counts,
        );

        let word_count = count_words(text);

        DocumentReport {
            word_count,
            diagnostics,
            category_counts,
            profile: profile.name.clone(),
        }
    }

    fn detect_puffery(
        &self,
        text: &str,
        sentences: &[(String, usize)],
        filtered: &DisabledRanges,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        if let Some(matcher) = &self.puffery_matcher {
            for mat in matcher.find_iter(text.as_bytes()) {
                if filtered.is_category_disabled(mat.start(), Category::Puffery) {
                    continue;
                }
                if !has_word_boundary(text, mat.start(), mat.end()) {
                    continue;
                }
                let snippet = slice_snippet(text, mat.start(), mat.end());
                if self.allow_phrase_set.contains(&snippet.to_lowercase()) {
                    continue;
                }
                let location = byte_to_location(text, mat.start());
                if filtered.is_line_ignored(location.line) {
                    continue;
                }
                diagnostics.push(Diagnostic {
                    category: Category::Puffery,
                    severity: Severity::Error,
                    message: format!("Puffery phrase detected: `{snippet}`"),
                    suggestion: Some("Replace with a concrete fact.".into()),
                    location,
                    span: (mat.start(), mat.end()),
                    snippet,
                });
                *counts.entry(Category::Puffery).or_default() += 1;
            }
        }

        if let Some(matcher) = &self.weasel_matcher {
            for mat in matcher.find_iter(text.as_bytes()) {
                if filtered.is_category_disabled(mat.start(), Category::Weasel) {
                    continue;
                }
                if !has_word_boundary(text, mat.start(), mat.end()) {
                    continue;
                }
                let snippet = slice_snippet(text, mat.start(), mat.end());
                if self.allow_phrase_set.contains(&snippet.to_lowercase()) {
                    continue;
                }
                let sentence_idx =
                    sentence_index_for_offset(sentences, mat.start()).unwrap_or(usize::MAX);
                if sentence_idx != usize::MAX && sentence_has_citation(&sentences[sentence_idx].0) {
                    continue;
                }
                let location = byte_to_location(text, mat.start());
                if filtered.is_line_ignored(location.line) {
                    continue;
                }
                diagnostics.push(Diagnostic {
                    category: Category::Weasel,
                    severity: Severity::Warning,
                    message: format!("Vague attribution: `{snippet}`"),
                    suggestion: Some("Name the specific source or remove.".into()),
                    location,
                    span: (mat.start(), mat.end()),
                    snippet,
                });
                *counts.entry(Category::Weasel).or_default() += 1;
            }
        }
    }

    fn detect_buzzwords(
        &self,
        text: &str,
        sentences: &[(String, usize)],
        filtered: &DisabledRanges,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        if let Some(matcher) = &self.buzzword_matcher {
            let mut hits: Vec<PhraseHit> = Vec::new();
            for mat in matcher.find_iter(text.as_bytes()) {
                if filtered.is_category_disabled(mat.start(), Category::Buzzword) {
                    continue;
                }
                if !has_word_boundary(text, mat.start(), mat.end()) {
                    continue;
                }
                let snippet = slice_snippet(text, mat.start(), mat.end());
                if self.allow_phrase_set.contains(&snippet.to_lowercase()) {
                    continue;
                }
                let suggestion = replacement_for(&snippet.to_lowercase());
                let sentence_idx =
                    sentence_index_for_offset(sentences, mat.start()).unwrap_or(usize::MAX);
                hits.push(PhraseHit {
                    start: mat.start(),
                    end: mat.end(),
                    snippet,
                    suggestion,
                    sentence_idx,
                });
            }
            if hits.is_empty() {
                return;
            }
            let mut grouped: HashMap<usize, Vec<PhraseHit>> = HashMap::new();
            for hit in hits {
                grouped.entry(hit.sentence_idx).or_default().push(hit);
            }
            for (sentence_idx, group) in grouped {
                let has_specifics = if sentence_idx != usize::MAX {
                    sentence_has_specifics(&sentences[sentence_idx].0)
                } else {
                    false
                };
                if has_specifics && group.len() == 1 {
                    continue;
                }
                let group_len = group.len();
                for hit in group {
                    let location = byte_to_location(text, hit.start);
                    if filtered.is_line_ignored(location.line) {
                        continue;
                    }
                    let sev = if group_len >= 2 {
                        Severity::Warning
                    } else {
                        Severity::Hint
                    };
                    diagnostics.push(Diagnostic {
                        category: Category::Buzzword,
                        severity: sev,
                        message: format!("Buzzword detected: `{}`", hit.snippet),
                        suggestion: hit.suggestion.clone(),
                        location,
                        span: (hit.start, hit.end),
                        snippet: hit.snippet.clone(),
                    });
                    *counts.entry(Category::Buzzword).or_default() += 1;
                }
            }
        }
    }

    fn detect_transitions(
        &self,
        text: &str,
        sentences: &[(String, usize)],
        filtered: &DisabledRanges,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        if let Some(matcher) = &self.transition_matcher {
            let mut hits: Vec<PhraseHit> = Vec::new();
            for mat in matcher.find_iter(text.as_bytes()) {
                if filtered.is_category_disabled(mat.start(), Category::Transition) {
                    continue;
                }
                if !has_word_boundary(text, mat.start(), mat.end()) {
                    continue;
                }
                let snippet = slice_snippet(text, mat.start(), mat.end());
                if self.allow_phrase_set.contains(&snippet.to_lowercase()) {
                    continue;
                }
                let sentence_idx =
                    sentence_index_for_offset(sentences, mat.start()).unwrap_or(usize::MAX);
                hits.push(PhraseHit {
                    start: mat.start(),
                    end: mat.end(),
                    snippet,
                    suggestion: Some("Trim or replace with a simple connector.".into()),
                    sentence_idx,
                });
            }
            if hits.is_empty() {
                return;
            }
            let mut grouped: HashMap<usize, Vec<PhraseHit>> = HashMap::new();
            for hit in hits {
                grouped.entry(hit.sentence_idx).or_default().push(hit);
            }
            for (sentence_idx, group) in grouped {
                let has_specifics = if sentence_idx != usize::MAX {
                    sentence_has_specifics(&sentences[sentence_idx].0)
                } else {
                    false
                };
                if has_specifics && group.len() == 1 {
                    continue;
                }
                let group_len = group.len();
                for hit in group {
                    let location = byte_to_location(text, hit.start);
                    if filtered.is_line_ignored(location.line) {
                        continue;
                    }
                    let sev = if group_len >= 2 {
                        Severity::Warning
                    } else {
                        Severity::Hint
                    };
                    diagnostics.push(Diagnostic {
                        category: Category::Transition,
                        severity: sev,
                        message: format!("Transitional filler detected: `{}`", hit.snippet),
                        suggestion: hit.suggestion.clone(),
                        location,
                        span: (hit.start, hit.end),
                        snippet: hit.snippet.clone(),
                    });
                    *counts.entry(Category::Transition).or_default() += 1;
                }
            }
        }
    }

    fn detect_marketing(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        if let Some(matcher) = &self.marketing_matcher {
            for mat in matcher.find_iter(text.as_bytes()) {
                if filtered.is_category_disabled(mat.start(), Category::Marketing) {
                    continue;
                }
                if !has_word_boundary(text, mat.start(), mat.end()) {
                    continue;
                }
                let snippet = slice_snippet(text, mat.start(), mat.end());
                if self.allow_phrase_set.contains(&snippet.to_lowercase()) {
                    continue;
                }
                let location = byte_to_location(text, mat.start());
                if filtered.is_line_ignored(location.line) {
                    continue;
                }
                diagnostics.push(Diagnostic {
                    category: Category::Marketing,
                    severity: Severity::Error,
                    message: format!("Marketing cliché detected: `{snippet}`"),
                    suggestion: Some("Swap for factual language.".into()),
                    location,
                    span: (mat.start(), mat.end()),
                    snippet,
                });
                *counts.entry(Category::Marketing).or_default() += 1;
            }
        }
    }

    fn detect_templates(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        profile: &ProfileRuntime,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        let iterators = [
            self.base_template_regexes.iter(),
            profile.template_regexes.iter(),
        ];
        for regex in iterators.into_iter().flatten() {
            for mat in regex.find_iter(text) {
                let snippet = slice_snippet(text, mat.start(), mat.end());
                if self.allow_phrase_set.contains(&snippet.to_lowercase()) {
                    continue;
                }
                let cat = if snippet.to_lowercase().contains("not") {
                    Category::NegativeParallel
                } else {
                    Category::Template
                };
                if filtered.is_category_disabled(mat.start(), cat) {
                    continue;
                }
                let location = byte_to_location(text, mat.start());
                if filtered.is_line_ignored(location.line) {
                    continue;
                }
                diagnostics.push(Diagnostic {
                    category: cat,
                    severity: Severity::Error,
                    message: format!("Template phrasing detected: `{snippet}`"),
                    suggestion: Some("Rewrite with direct language.".into()),
                    location,
                    span: (mat.start(), mat.end()),
                    snippet,
                });
                *counts.entry(cat).or_default() += 1;
            }
        }
    }

    fn detect_connectors(
        &self,
        text: &str,
        sentences: &[(String, usize)],
        filtered: &DisabledRanges,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        let connectors = [
            "however",
            "furthermore",
            "moreover",
            "nevertheless",
            "nonetheless",
            "consequently",
            "therefore",
            "thus",
            "accordingly",
            "as a result",
            "in addition",
            "at the same time",
        ];

        for (sentence, offset) in sentences {
            if filtered.is_category_disabled(*offset, Category::ConnectorGlut) {
                continue;
            }
            let lower = sentence.to_lowercase();
            let mut count = 0;
            for connector in connectors.iter() {
                count += lower.matches(connector).count();
            }
            if count > self.config.limits.connectors_per_sentence {
                let location = byte_to_location(text, *offset);
                if filtered.is_line_ignored(location.line) {
                    continue;
                }
                diagnostics.push(Diagnostic {
                    category: Category::ConnectorGlut,
                    severity: Severity::Warning,
                    message: format!(
                        "Sentence uses {} connectors; limit is {}.",
                        count, self.config.limits.connectors_per_sentence
                    ),
                    suggestion: Some("Split the sentence or drop extra connectors.".into()),
                    location,
                    span: (*offset, *offset + sentence.len()),
                    snippet: sentence.trim().to_string(),
                });
                *counts.entry(Category::ConnectorGlut).or_default() += 1;
            }
        }
    }

    fn detect_ranges(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        for mat in self.range_regex.find_iter(text) {
            if filtered.is_category_disabled(mat.start(), Category::Weasel) {
                continue;
            }
            let snippet = slice_snippet(text, mat.start(), mat.end());
            let location = byte_to_location(text, mat.start());
            if filtered.is_line_ignored(location.line) {
                continue;
            }
            diagnostics.push(Diagnostic {
                category: Category::Weasel,
                severity: Severity::Warning,
                message: format!("Exaggerated range detected: `{snippet}`"),
                suggestion: Some("List the specific items or tighten the range.".into()),
                location,
                span: (mat.start(), mat.end()),
                snippet,
            });
            *counts.entry(Category::Weasel).or_default() += 1;
        }
    }

    fn detect_question_lead(
        &self,
        text: &str,
        sentences: &[(String, usize)],
        filtered: &DisabledRanges,
        profile: &ProfileRuntime,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        let Some(limit) = profile.question_lead_limit else {
            return;
        };
        let mut question_count = 0usize;
        let mut first_question_offset = None;
        let mut first_question_snippet = String::new();

        for (sentence, offset) in sentences {
            if filtered.is_category_disabled(*offset, Category::Tone) {
                continue;
            }
            let trimmed = sentence.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with('#') || trimmed.starts_with('-') || trimmed.starts_with('*') {
                continue;
            }
            if trimmed.len() < 2 {
                continue;
            }
            let is_question = trimmed.ends_with('?');
            if is_question {
                question_count += 1;
                if first_question_offset.is_none() {
                    first_question_offset = Some(*offset);
                    first_question_snippet = sentence.trim().to_string();
                }
                continue;
            }
            break;
        }

        if question_count > limit {
            if let Some(start) = first_question_offset {
                let location = byte_to_location(text, start);
                if filtered.is_line_ignored(location.line) {
                    return;
                }
                diagnostics.push(Diagnostic {
                    category: Category::Tone,
                    severity: Severity::Hint,
                    message: format!(
                        "Intro uses {} consecutive questions; limit is {}.",
                        question_count, limit
                    ),
                    suggestion: Some("Replace question lead with a concise statement.".into()),
                    location,
                    span: (start, start + first_question_snippet.len()),
                    snippet: first_question_snippet.clone(),
                });
                *counts.entry(Category::Tone).or_default() += 1;
            }
        }
    }

    fn detect_mid_sentence_questions(
        &self,
        text: &str,
        sentences: &[(String, usize)],
        filtered: &DisabledRanges,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        for mat in MID_SENTENCE_QUESTION_RE.find_iter(text) {
            if filtered.is_category_disabled(mat.start(), Category::Tone) {
                continue;
            }
            let location = byte_to_location(text, mat.start());
            if filtered.is_line_ignored(location.line) {
                continue;
            }
            let snippet = sentence_index_for_offset(sentences, mat.start())
                .map(|idx| sentences[idx].0.trim().to_string())
                .unwrap_or_else(|| slice_snippet(text, mat.start(), mat.end()));
            diagnostics.push(Diagnostic {
                category: Category::Tone,
                severity: Severity::Hint,
                message: "Mid-sentence question detected.".into(),
                suggestion: Some("Rewrite as a statement or split into two sentences.".into()),
                location,
                span: (mat.start(), mat.end()),
                snippet,
            });
            *counts.entry(Category::Tone).or_default() += 1;
        }
    }

    fn detect_exclamation_density(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        profile: &ProfileRuntime,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        let Some(limit) = profile.max_exclamations_per_paragraph else {
            return;
        };
        if limit == 0 {
            return;
        }

        let mut offset = 0usize;
        let mut paragraph = String::new();
        let mut paragraph_start = 0usize;

        for segment in text.split_inclusive('\n') {
            let trimmed = segment.trim();
            let is_blank = trimmed.is_empty();
            if paragraph.is_empty() {
                paragraph_start = offset;
            }
            if !is_blank {
                paragraph.push_str(segment);
            }
            if is_blank {
                self.flush_paragraph_exclamations(
                    text,
                    &paragraph,
                    paragraph_start,
                    filtered,
                    limit,
                    diagnostics,
                    counts,
                );
                paragraph.clear();
            }
            offset += segment.len();
        }

        if !paragraph.trim().is_empty() {
            self.flush_paragraph_exclamations(
                text,
                &paragraph,
                paragraph_start,
                filtered,
                limit,
                diagnostics,
                counts,
            );
        }
    }

    fn flush_paragraph_exclamations(
        &self,
        text: &str,
        paragraph: &str,
        start: usize,
        filtered: &DisabledRanges,
        limit: usize,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        if paragraph.trim().is_empty() {
            return;
        }
        if filtered.is_category_disabled(start, Category::Tone) {
            return;
        }
        let count = paragraph.matches('!').count();
        if count > limit {
            let relative = paragraph.find('!').unwrap_or(0);
            let location = byte_to_location(text, start + relative);
            if filtered.is_line_ignored(location.line) {
                return;
            }
            diagnostics.push(Diagnostic {
                category: Category::Tone,
                severity: Severity::Hint,
                message: format!(
                    "Paragraph contains {} exclamation marks; limit is {}.",
                    count, limit
                ),
                suggestion: Some("Reduce promotional punctuation.".into()),
                location,
                span: (start, start + paragraph.len()),
                snippet: paragraph.trim().to_string(),
            });
            *counts.entry(Category::Tone).or_default() += 1;
        }
    }

    fn detect_sentence_length(
        &self,
        text: &str,
        sentences: &[(String, usize)],
        filtered: &DisabledRanges,
        profile: &ProfileRuntime,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        let Some(limit) = profile.max_sentence_length else {
            return;
        };
        for (sentence, offset) in sentences {
            if filtered.is_category_disabled(*offset, Category::SentenceLength) {
                continue;
            }
            let word_count = sentence.split_whitespace().count();
            if word_count > limit {
                let location = byte_to_location(text, *offset);
                if filtered.is_line_ignored(location.line) {
                    continue;
                }
                diagnostics.push(Diagnostic {
                    category: Category::SentenceLength,
                    severity: Severity::Hint,
                    message: format!(
                        "Sentence length {} exceeds limit of {} words.",
                        word_count, limit
                    ),
                    suggestion: Some("Split into shorter sentences.".into()),
                    location,
                    span: (*offset, *offset + sentence.len()),
                    snippet: sentence.trim().to_string(),
                });
                *counts.entry(Category::SentenceLength).or_default() += 1;
            }
        }
    }

    fn detect_repetition(
        &self,
        text: &str,
        sentences: &[(String, usize)],
        filtered: &DisabledRanges,
        profile: &ProfileRuntime,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        if profile.max_duplicate_sentences == 0 {
            return;
        }
        let mut seen: HashMap<String, usize> = HashMap::new();
        for (sentence, offset) in sentences {
            if filtered.is_category_disabled(*offset, Category::Repetition) {
                continue;
            }
            let normalised = normalize_sentence(sentence);
            if normalised.len() < 12 {
                continue;
            }
            let entry = seen.entry(normalised).and_modify(|c| *c += 1).or_insert(1);
            if *entry > profile.max_duplicate_sentences {
                let location = byte_to_location(text, *offset);
                if filtered.is_line_ignored(location.line) {
                    continue;
                }
                diagnostics.push(Diagnostic {
                    category: Category::Repetition,
                    severity: Severity::Warning,
                    message: "Sentence repeats earlier phrasing.".into(),
                    suggestion: Some("Introduce new detail or remove duplicates.".into()),
                    location,
                    span: (*offset, *offset + sentence.len()),
                    snippet: sentence.trim().to_string(),
                });
                *counts.entry(Category::Repetition).or_default() += 1;
            }
        }
    }

    fn detect_cadence(
        &self,
        text: &str,
        sentences: &[(String, usize)],
        filtered: &DisabledRanges,
        profile: &ProfileRuntime,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        if profile.cadence_starts.is_empty() || profile.cadence_limit == 0 {
            return;
        }
        let mut previous: Option<String> = None;
        let mut streak = 0usize;

        for (sentence, offset) in sentences {
            if filtered.is_category_disabled(*offset, Category::Cadence) {
                continue;
            }
            let trimmed = sentence.trim_start();
            if trimmed.is_empty() {
                previous = None;
                streak = 0;
                continue;
            }
            if trimmed.starts_with('#') || trimmed.starts_with('-') || trimmed.starts_with('*') {
                previous = None;
                streak = 0;
                continue;
            }
            if let Some(first) = trimmed.split_whitespace().next() {
                let cleaned = first
                    .trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase();
                if profile.cadence_starts.contains(&cleaned) {
                    if let Some(prev) = &previous {
                        if prev == &cleaned {
                            streak += 1;
                        } else {
                            previous = Some(cleaned.clone());
                            streak = 1;
                        }
                    } else {
                        previous = Some(cleaned.clone());
                        streak = 1;
                    }
                    if streak > profile.cadence_limit {
                        let location = byte_to_location(text, *offset);
                        if filtered.is_line_ignored(location.line) {
                            continue;
                        }
                        diagnostics.push(Diagnostic {
                            category: Category::Cadence,
                            severity: Severity::Hint,
                            message: format!(
                                "Cadence repeats opening `{}` more than {} times in a row.",
                                first.trim_matches(|c: char| !c.is_alphanumeric()),
                                profile.cadence_limit
                            ),
                            suggestion: Some("Vary sentence openings to avoid monotony.".into()),
                            location,
                            span: (*offset, *offset + sentence.len()),
                            snippet: sentence.trim().to_string(),
                        });
                        *counts.entry(Category::Cadence).or_default() += 1;
                    }
                    continue;
                }
            }
            previous = None;
            streak = 0;
        }
    }

    fn detect_broad_terms(
        &self,
        text: &str,
        sentences: &[(String, usize)],
        filtered: &DisabledRanges,
        profile: &ProfileRuntime,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        if profile.broad_terms.is_empty() {
            return;
        }
        for (sentence, offset) in sentences {
            if filtered.is_category_disabled(*offset, Category::BroadTerm) {
                continue;
            }
            let lower = sentence.to_lowercase();
            if !lower.chars().any(|c| c.is_alphabetic()) {
                continue;
            }
            if lower.starts_with('#') {
                continue;
            }
            if sentence_has_specifics(sentence) {
                continue;
            }
            if let Some(term) = profile
                .broad_terms
                .iter()
                .find(|term| find_term_with_boundary(&lower, term).is_some())
            {
                let location = byte_to_location(text, *offset);
                if filtered.is_line_ignored(location.line) {
                    continue;
                }
                diagnostics.push(Diagnostic {
                    category: Category::BroadTerm,
                    severity: Severity::Hint,
                    message: format!("Broad term `{}` detected without specifics.", term),
                    suggestion: Some("Replace with a concrete description.".into()),
                    location,
                    span: (*offset, *offset + sentence.len()),
                    snippet: sentence.trim().to_string(),
                });
                *counts.entry(Category::BroadTerm).or_default() += 1;
            }
        }
    }

    fn detect_call_to_action(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        profile: &ProfileRuntime,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        if let Some(matcher) = &profile.call_to_action_matcher {
            for mat in matcher.find_iter(text.as_bytes()) {
                if filtered.is_category_disabled(mat.start(), Category::CallToAction) {
                    continue;
                }
                if !has_word_boundary(text, mat.start(), mat.end()) {
                    continue;
                }
                let snippet = slice_snippet(text, mat.start(), mat.end());
                let location = byte_to_location(text, mat.start());
                if filtered.is_line_ignored(location.line) {
                    continue;
                }
                diagnostics.push(Diagnostic {
                    category: Category::CallToAction,
                    severity: Severity::Warning,
                    message: format!("Call-to-action template detected: `{snippet}`"),
                    suggestion: Some("Use a direct statement instead of marketing CTA.".into()),
                    location,
                    span: (mat.start(), mat.end()),
                    snippet,
                });
                *counts.entry(Category::CallToAction).or_default() += 1;
            }
        }
    }

    fn detect_confidence(
        &self,
        text: &str,
        sentences: &[(String, usize)],
        filtered: &DisabledRanges,
        profile: &ProfileRuntime,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        let mut flagged: HashSet<usize> = HashSet::new();

        if let Some(matcher) = &profile.confidence_matcher {
            for mat in matcher.find_iter(text.as_bytes()) {
                let start = mat.start();
                if filtered.is_category_disabled(start, Category::Confidence)
                    || flagged.contains(&start)
                {
                    continue;
                }
                if !has_word_boundary(text, start, mat.end()) {
                    continue;
                }
                let sentence_idx =
                    sentence_index_for_offset(sentences, start).unwrap_or(usize::MAX);
                if sentence_idx != usize::MAX && sentence_has_citation(&sentences[sentence_idx].0) {
                    continue;
                }
                let snippet = slice_snippet(text, start, mat.end());
                let location = byte_to_location(text, start);
                if filtered.is_line_ignored(location.line) {
                    continue;
                }
                diagnostics.push(Diagnostic {
                    category: Category::Confidence,
                    severity: Severity::Warning,
                    message: format!("Confidence claim `{snippet}` detected without evidence."),
                    suggestion: Some("Provide a source or remove the claim.".into()),
                    location,
                    span: (start, mat.end()),
                    snippet,
                });
                *counts.entry(Category::Confidence).or_default() += 1;
                flagged.insert(start);
            }
        }

        if profile.detect_percent_claims {
            for mat in CONFIDENCE_PERCENT_RE.find_iter(text) {
                let start = mat.start();
                if filtered.is_category_disabled(start, Category::Confidence)
                    || flagged.contains(&start)
                {
                    continue;
                }
                let sentence_idx =
                    sentence_index_for_offset(sentences, start).unwrap_or(usize::MAX);
                if sentence_idx != usize::MAX && sentence_has_citation(&sentences[sentence_idx].0) {
                    continue;
                }
                if sentence_idx != usize::MAX
                    && percent_claim_is_contextual(&sentences[sentence_idx].0)
                {
                    continue;
                }
                let end = mat.end();
                let snippet = slice_snippet(text, start, end);
                let location = byte_to_location(text, start);
                if filtered.is_line_ignored(location.line) {
                    continue;
                }
                diagnostics.push(Diagnostic {
                    category: Category::Confidence,
                    severity: Severity::Warning,
                    message: format!(
                        "Numeric confidence `{snippet}` detected without supporting context."
                    ),
                    suggestion: Some("Explain the statistic or remove it.".into()),
                    location,
                    span: (start, end),
                    snippet,
                });
                *counts.entry(Category::Confidence).or_default() += 1;
                flagged.insert(start);
            }
        }
    }

    fn detect_required_patterns(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        profile: &ProfileRuntime,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        let Some(anchor) = analysis_anchor_offset(filtered, text) else {
            return;
        };
        if filtered.is_category_disabled(anchor, Category::Structure) {
            return;
        }
        let location = byte_to_location(text, anchor);
        if filtered.is_line_ignored(location.line) {
            return;
        }
        for regex in &profile.required_patterns {
            if regex.find(text).is_none() {
                diagnostics.push(Diagnostic {
                    category: Category::Structure,
                    severity: Severity::Warning,
                    message: format!("Required pattern `{}` not found.", regex.as_str()),
                    suggestion: Some("Add the missing section or reference.".into()),
                    location: location.clone(),
                    span: (anchor, anchor),
                    snippet: String::new(),
                });
                *counts.entry(Category::Structure).or_default() += 1;
            }
        }
    }

    fn detect_forbidden_patterns(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        profile: &ProfileRuntime,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        for regex in &profile.forbidden_patterns {
            for mat in regex.find_iter(text) {
                if filtered.is_category_disabled(mat.start(), Category::Structure) {
                    continue;
                }
                let location = byte_to_location(text, mat.start());
                if filtered.is_line_ignored(location.line) {
                    continue;
                }
                diagnostics.push(Diagnostic {
                    category: Category::Structure,
                    severity: Severity::Warning,
                    message: format!("Forbidden pattern `{}` detected.", regex.as_str()),
                    suggestion: Some("Remove or rewrite the offending section.".into()),
                    location,
                    span: (mat.start(), mat.end()),
                    snippet: slice_snippet(text, mat.start(), mat.end()),
                });
                *counts.entry(Category::Structure).or_default() += 1;
            }
        }
    }

    fn detect_bullet_items(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        profile: &ProfileRuntime,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        let Some(limit) = profile.max_bullet_items else {
            return;
        };

        let mut current = 0usize;
        let mut start_line = 0usize;
        let mut start_offset = 0usize;

        for (idx, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            let is_bullet =
                trimmed.starts_with("- ") || trimmed.starts_with("* ") || is_numbered_list(trimmed);
            if is_bullet {
                let offset = line_offset(text, idx);
                if filtered.is_category_disabled(offset, Category::Structure) {
                    continue;
                }
                if current == 0 {
                    start_line = idx + 1;
                    start_offset = offset;
                }
                current += 1;
            } else if current > 0 {
                if current > limit {
                    let location = byte_to_location(text, start_offset);
                    if filtered.is_line_ignored(location.line) {
                        current = 0;
                        continue;
                    }
                    diagnostics.push(Diagnostic {
                        category: Category::Structure,
                        severity: Severity::Hint,
                        message: format!("List contains {} items; limit is {}.", current, limit),
                        suggestion: Some("Break long lists into sub-sections.".into()),
                        location,
                        span: (start_offset, start_offset + line.len()),
                        snippet: text
                            .lines()
                            .skip(start_line - 1)
                            .take(current)
                            .collect::<Vec<_>>()
                            .join("\n"),
                    });
                    *counts.entry(Category::Structure).or_default() += 1;
                }
                current = 0;
            }
        }

        if current > limit {
            let location = byte_to_location(text, start_offset);
            if filtered.is_line_ignored(location.line) {
                return;
            }
            diagnostics.push(Diagnostic {
                category: Category::Structure,
                severity: Severity::Hint,
                message: format!("List contains {} items; limit is {}.", current, limit),
                suggestion: Some("Break long lists into sub-sections.".into()),
                location,
                span: (start_offset, start_offset + text.len()),
                snippet: String::new(),
            });
            *counts.entry(Category::Structure).or_default() += 1;
        }
    }

    fn detect_emoji_bullets(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        for (idx, line) in text.lines().enumerate() {
            let offset = line_offset(text, idx);
            if filtered.is_category_disabled(offset, Category::Formatting) {
                continue;
            }
            let trimmed = line.trim_start();
            let content = if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
                trimmed[2..].trim_start()
            } else if is_numbered_list(trimmed) {
                let mut chars = trimmed.char_indices();
                let mut start = None;
                while let Some((i, ch)) = chars.next() {
                    if ch.is_ascii_digit() || ch == '.' || ch == ')' {
                        continue;
                    }
                    if ch.is_whitespace() {
                        continue;
                    }
                    start = Some(i);
                    break;
                }
                match start {
                    Some(i) => &trimmed[i..],
                    None => continue,
                }
            } else {
                continue;
            };
            if let Some(first) = content.trim_start().chars().next() {
                if is_emoji_hint(first) {
                    let location = byte_to_location(text, offset);
                    if filtered.is_line_ignored(location.line) {
                        continue;
                    }
                    diagnostics.push(Diagnostic {
                        category: Category::Formatting,
                        severity: Severity::Hint,
                        message: "Emoji-led bullet detected.".into(),
                        suggestion: Some("Use plain text bullets to reduce stylized noise.".into()),
                        location,
                        span: (offset, offset + line.len()),
                        snippet: line.to_string(),
                    });
                    *counts.entry(Category::Formatting).or_default() += 1;
                }
            }
        }
    }

    fn detect_bold_lead_bullets(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        let limit = self.config.limits.bold_lead_bullets_per_list;
        if limit == 0 {
            return;
        }

        let mut in_list = false;
        let mut bold_leads = 0usize;
        let mut list_start = 0usize;
        let mut list_end = 0usize;
        let mut snippet_lines: Vec<String> = Vec::new();

        let flush = |in_list: &mut bool,
                     bold_leads: &mut usize,
                     list_start: &mut usize,
                     list_end: &mut usize,
                     snippet_lines: &mut Vec<String>,
                     filtered: &DisabledRanges,
                     diagnostics: &mut Vec<Diagnostic>,
                     counts: &mut BTreeMap<Category, usize>,
                     text: &str,
                     limit: usize| {
            if *in_list && *bold_leads >= limit {
                if !filtered.is_category_disabled(*list_start, Category::Formatting) {
                    let location = byte_to_location(text, *list_start);
                    if !filtered.is_line_ignored(location.line) {
                        diagnostics.push(Diagnostic {
                            category: Category::Formatting,
                            severity: Severity::Hint,
                            message: format!(
                                "List uses {} bold-led bullets; limit is {}.",
                                *bold_leads, limit
                            ),
                            suggestion: Some("Use plain bullets or reduce bold lead-ins.".into()),
                            location,
                            span: (*list_start, (*list_end).max(*list_start)),
                            snippet: snippet_lines.join("\n"),
                        });
                        *counts.entry(Category::Formatting).or_default() += 1;
                    }
                }
            }
            *in_list = false;
            *bold_leads = 0;
            *list_start = 0;
            *list_end = 0;
            snippet_lines.clear();
        };

        for (idx, line) in text.lines().enumerate() {
            let offset = line_offset(text, idx);
            if filtered.is_category_disabled(offset, Category::Formatting) {
                flush(
                    &mut in_list,
                    &mut bold_leads,
                    &mut list_start,
                    &mut list_end,
                    &mut snippet_lines,
                    filtered,
                    diagnostics,
                    counts,
                    text,
                    limit,
                );
                continue;
            }

            let trimmed_full = line.trim();
            if trimmed_full.starts_with("<!--")
                && trimmed_full.ends_with("-->")
                && trimmed_full.contains("dwg:")
            {
                continue;
            }

            let trimmed = line.trim_start();
            if trimmed.is_empty() {
                flush(
                    &mut in_list,
                    &mut bold_leads,
                    &mut list_start,
                    &mut list_end,
                    &mut snippet_lines,
                    filtered,
                    diagnostics,
                    counts,
                    text,
                    limit,
                );
                continue;
            }

            let content = if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
                Some(trimmed[2..].trim_start())
            } else if is_numbered_list(trimmed) {
                let mut chars = trimmed.char_indices();
                let mut start = None;
                while let Some((i, ch)) = chars.next() {
                    if ch.is_ascii_digit() || ch == '.' || ch == ')' {
                        continue;
                    }
                    if ch.is_whitespace() {
                        continue;
                    }
                    start = Some(i);
                    break;
                }
                start.map(|i| trimmed[i..].trim_start())
            } else {
                None
            };

            let Some(content) = content else {
                flush(
                    &mut in_list,
                    &mut bold_leads,
                    &mut list_start,
                    &mut list_end,
                    &mut snippet_lines,
                    filtered,
                    diagnostics,
                    counts,
                    text,
                    limit,
                );
                continue;
            };

            if !in_list {
                in_list = true;
                list_start = offset;
                snippet_lines.clear();
                bold_leads = 0;
            }
            list_end = offset + line.len();
            if snippet_lines.len() < 8 {
                snippet_lines.push(line.to_string());
            }
            let content_trimmed = content.trim_start();
            if !filtered.is_line_ignored(idx + 1)
                && (content_trimmed.starts_with("**") || content_trimmed.starts_with("__"))
            {
                bold_leads += 1;
            }
        }

        flush(
            &mut in_list,
            &mut bold_leads,
            &mut list_start,
            &mut list_end,
            &mut snippet_lines,
            filtered,
            diagnostics,
            counts,
            text,
            limit,
        );
    }

    fn detect_rule_of_three(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        for (paragraph, offset) in split_paragraphs_with_offset(text) {
            if filtered.is_category_disabled(offset, Category::RuleOfThree) {
                continue;
            }
            if paragraph.trim().is_empty() {
                continue;
            }
            let mut seen = 0;
            for mat in self.rule_of_three_regex.find_iter(paragraph) {
                let m_start = offset + mat.start();
                if filtered.is_category_disabled(m_start, Category::RuleOfThree) {
                    continue;
                }
                seen += 1;
                if seen > self.config.limits.rule_of_three_per_paragraph {
                    let snippet = slice_snippet(text, m_start, m_start + mat.as_str().len());
                    let location = byte_to_location(text, m_start);
                    if filtered.is_line_ignored(location.line) {
                        continue;
                    }
                    diagnostics.push(Diagnostic {
                        category: Category::RuleOfThree,
                        severity: Severity::Warning,
                        message: format!("Rule-of-three phrasing detected: `{snippet}`"),
                        suggestion: Some("Reduce to the single concrete item that matters.".into()),
                        location,
                        span: (m_start, m_start + mat.as_str().len()),
                        snippet,
                    });
                    *counts.entry(Category::RuleOfThree).or_default() += 1;
                }
            }
        }
    }

    fn detect_em_dash(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        for (paragraph, offset) in split_paragraphs_with_offset(text) {
            if filtered.is_category_disabled(offset, Category::EmDash) {
                continue;
            }
            if paragraph.trim().is_empty() {
                continue;
            }
            let occurrences = paragraph.matches('—').count();
            if occurrences > self.config.limits.em_dashes_per_paragraph {
                let location = byte_to_location(text, offset);
                if filtered.is_line_ignored(location.line) {
                    continue;
                }
                diagnostics.push(Diagnostic {
                    category: Category::EmDash,
                    severity: Severity::Hint,
                    message: format!(
                        "Paragraph contains {} em dashes; limit is {}.",
                        occurrences, self.config.limits.em_dashes_per_paragraph
                    ),
                    suggestion: Some("Swap extra em dashes for commas or periods.".into()),
                    location,
                    span: (offset, offset + paragraph.len()),
                    snippet: paragraph.trim().to_string(),
                });
                *counts.entry(Category::EmDash).or_default() += 1;
            }
        }
    }

    fn detect_bold_spans(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        let limit = self.config.limits.bold_spans_per_paragraph;
        if limit == 0 {
            return;
        }
        for (paragraph, offset) in split_paragraphs_with_offset(text) {
            if paragraph.trim().is_empty() {
                continue;
            }
            if filtered.is_category_disabled(offset, Category::Formatting) {
                continue;
            }
            let mut count = 0usize;
            for mat in BOLD_SPAN_RE.find_iter(paragraph) {
                let abs = offset + mat.start();
                if filtered.is_category_disabled(abs, Category::Formatting) {
                    continue;
                }
                count += 1;
            }
            if count > limit {
                let location = byte_to_location(text, offset);
                if filtered.is_line_ignored(location.line) {
                    continue;
                }
                diagnostics.push(Diagnostic {
                    category: Category::Formatting,
                    severity: Severity::Hint,
                    message: format!("Paragraph uses {} bold spans; limit is {}.", count, limit),
                    suggestion: Some("Use bold sparingly or convert to plain labels.".into()),
                    location,
                    span: (offset, offset + paragraph.len()),
                    snippet: paragraph.trim().to_string(),
                });
                *counts.entry(Category::Formatting).or_default() += 1;
            }
        }
    }

    fn detect_headings(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        profile: &ProfileRuntime,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        let mut captures: Vec<HeadingCapture> = Vec::new();

        for (idx, line) in text.lines().enumerate() {
            let offset = line_offset(text, idx);
            if filtered.is_disabled(offset) {
                continue;
            }
            if !line.trim_start().starts_with('#') {
                continue;
            }
            let content = line.trim_start_matches('#').trim();
            if content.is_empty() {
                continue;
            }
            let level = line.chars().take_while(|c| *c == '#').count();
            let capture = HeadingCapture {
                line: idx + 1,
                column: 1,
                offset,
                len: line.len(),
                text: line.to_string(),
                lower: content.to_lowercase(),
            };
            captures.push(capture);

            if let Some(max_depth) = profile.max_heading_depth {
                if level > max_depth
                    && !filtered.is_category_disabled(offset, Category::Structure)
                    && !filtered.is_line_ignored(idx + 1)
                {
                    diagnostics.push(Diagnostic {
                        category: Category::Structure,
                        severity: Severity::Hint,
                        message: format!("Heading depth {} exceeds limit {}.", level, max_depth),
                        suggestion: Some("Flatten heading structure or use fewer levels.".into()),
                        location: Location {
                            line: idx + 1,
                            column: 1,
                        },
                        span: (offset, offset + line.len()),
                        snippet: line.to_string(),
                    });
                    *counts.entry(Category::Structure).or_default() += 1;
                }
            }

            if profile.forbid_rhetorical_headings
                && content.ends_with('?')
                && !filtered.is_category_disabled(offset, Category::Structure)
                && !filtered.is_line_ignored(idx + 1)
            {
                diagnostics.push(Diagnostic {
                    category: Category::Structure,
                    severity: Severity::Hint,
                    message: format!("Rhetorical heading detected: `{}`", content),
                    suggestion: Some("Use a declarative heading.".into()),
                    location: Location {
                        line: idx + 1,
                        column: 1,
                    },
                    span: (offset, offset + line.len()),
                    snippet: line.to_string(),
                });
                *counts.entry(Category::Structure).or_default() += 1;
            }

            if content.chars().any(is_emoji_hint)
                && !filtered.is_category_disabled(offset, Category::Formatting)
                && !filtered.is_line_ignored(idx + 1)
            {
                diagnostics.push(Diagnostic {
                    category: Category::Formatting,
                    severity: Severity::Hint,
                    message: format!("Emoji found in heading: `{content}`"),
                    suggestion: Some("Remove emoji from headings.".into()),
                    location: Location {
                        line: idx + 1,
                        column: 1,
                    },
                    span: (offset, offset + line.len()),
                    snippet: line.to_string(),
                });
                *counts.entry(Category::Formatting).or_default() += 1;
            }

            if matches_bold_list(line)
                && !filtered.is_category_disabled(offset, Category::Formatting)
                && !filtered.is_line_ignored(idx + 1)
            {
                diagnostics.push(Diagnostic {
                    category: Category::Formatting,
                    severity: Severity::Hint,
                    message: "Bold list heading detected".into(),
                    suggestion: Some("Use plain bullet labels instead of bold sentences.".into()),
                    location: Location {
                        line: idx + 1,
                        column: 1,
                    },
                    span: (offset, offset + line.len()),
                    snippet: line.to_string(),
                });
                *counts.entry(Category::Formatting).or_default() += 1;
            }

            if self.config.heading_style == HeadingStyle::SentenceCase
                && appears_title_case(content)
                && !filtered.is_category_disabled(offset, Category::Formatting)
                && !filtered.is_line_ignored(idx + 1)
            {
                diagnostics.push(Diagnostic {
                    category: Category::Formatting,
                    severity: Severity::Hint,
                    message: format!("Heading should be sentence case: `{content}`"),
                    suggestion: Some("Lowercase the remaining words.".into()),
                    location: Location {
                        line: idx + 1,
                        column: 1,
                    },
                    span: (offset, offset + line.len()),
                    snippet: line.to_string(),
                });
                *counts.entry(Category::Formatting).or_default() += 1;
            }
        }

        if let Some(max) = profile.max_headings {
            if captures.len() > max {
                let capture = captures
                    .get(max)
                    .or_else(|| captures.last())
                    .cloned()
                    .unwrap_or(HeadingCapture {
                        line: 1,
                        column: 1,
                        offset: 0,
                        len: 0,
                        text: String::new(),
                        lower: String::new(),
                    });
                if !filtered.is_category_disabled(capture.offset, Category::Structure)
                    && !filtered.is_line_ignored(capture.line)
                {
                    diagnostics.push(Diagnostic {
                        category: Category::Structure,
                        severity: Severity::Warning,
                        message: format!(
                            "Document has {} headings; limit is {}.",
                            captures.len(),
                            max
                        ),
                        suggestion: Some("Consolidate sections or reduce heading depth.".into()),
                        location: Location {
                            line: capture.line,
                            column: capture.column,
                        },
                        span: (capture.offset, capture.offset + capture.len),
                        snippet: capture.text,
                    });
                    *counts.entry(Category::Structure).or_default() += 1;
                }
            }
        }

        let lower_headings: HashSet<String> = captures.iter().map(|c| c.lower.clone()).collect();
        for required in &profile.required_headings {
            if !lower_headings.contains(required) {
                let Some(anchor) = analysis_anchor_offset(filtered, text) else {
                    continue;
                };
                if filtered.is_category_disabled(anchor, Category::Structure) {
                    continue;
                }
                let location = byte_to_location(text, anchor);
                if filtered.is_line_ignored(location.line) {
                    continue;
                }
                diagnostics.push(Diagnostic {
                    category: Category::Structure,
                    severity: Severity::Warning,
                    message: format!("Required heading `{required}` is missing."),
                    suggestion: Some("Add the required section heading.".into()),
                    location,
                    span: (anchor, anchor),
                    snippet: String::new(),
                });
                *counts.entry(Category::Structure).or_default() += 1;
            }
        }

        for regex in &profile.banned_heading_regexes {
            for capture in &captures {
                if regex.is_match(&capture.text) {
                    if filtered.is_category_disabled(capture.offset, Category::Structure)
                        || filtered.is_line_ignored(capture.line)
                    {
                        continue;
                    }
                    diagnostics.push(Diagnostic {
                        category: Category::Structure,
                        severity: Severity::Warning,
                        message: format!(
                            "Heading `{}` matches disallowed pattern `{}`.",
                            capture.text.trim(),
                            regex.as_str()
                        ),
                        suggestion: Some("Rename or remove the heading.".into()),
                        location: Location {
                            line: capture.line,
                            column: capture.column,
                        },
                        span: (capture.offset, capture.offset + capture.len),
                        snippet: capture.text.clone(),
                    });
                    *counts.entry(Category::Structure).or_default() += 1;
                }
            }
        }
    }

    fn detect_quotes(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        if self.config.quote_style != QuoteStyle::Straight {
            return;
        }
        let curly_chars = ['“', '”', '‘', '’'];
        for (idx, ch) in text.char_indices() {
            if filtered.is_category_disabled(idx, Category::QuoteStyle) {
                continue;
            }
            if curly_chars.contains(&ch) {
                let location = byte_to_location(text, idx);
                if filtered.is_line_ignored(location.line) {
                    continue;
                }
                diagnostics.push(Diagnostic {
                    category: Category::QuoteStyle,
                    severity: Severity::Hint,
                    message: "Curly quotation detected; prefer straight quotes".into(),
                    suggestion: Some("Replace with ' or \".".into()),
                    location,
                    span: (idx, idx + ch.len_utf8()),
                    snippet: ch.to_string(),
                });
                *counts.entry(Category::QuoteStyle).or_default() += 1;
            }
        }
    }

    /// Detect statistical indicators of AI-generated text:
    /// - Low sentence length variance (AI writes very uniform sentences)
    /// - Repeated sentence openings (AI recycles structures)
    /// - High passive voice density
    fn detect_statistical_slop(
        &self,
        text: &str,
        sentences: &[(String, usize)],
        filtered: &DisabledRanges,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        // Need at least 5 sentences for meaningful statistics
        if sentences.len() < 5 {
            return;
        }

        let Some(anchor) = analysis_anchor_offset(filtered, text) else {
            return;
        };
        let anchor_location = byte_to_location(text, anchor);

        let tone_enabled = !filtered.is_category_disabled(anchor, Category::Tone)
            && !filtered.is_line_ignored(anchor_location.line);
        let cadence_enabled = !filtered.is_category_disabled(anchor, Category::Cadence)
            && !filtered.is_line_ignored(anchor_location.line);

        if tone_enabled {
            // Calculate sentence length statistics
            let lengths: Vec<usize> = sentences
                .iter()
                .filter(|(s, off)| {
                    !filtered.is_category_disabled(*off, Category::Tone)
                        && !s.trim().starts_with('#')
                })
                .map(|(s, _)| s.split_whitespace().count())
                .filter(|&len| len >= 3) // Skip very short "sentences"
                .collect();

            if lengths.len() >= 5 {
                let avg_len = lengths.iter().sum::<usize>() as f32 / lengths.len() as f32;
                let variance = lengths
                    .iter()
                    .map(|&len| {
                        let diff = len as f32 - avg_len;
                        diff * diff
                    })
                    .sum::<f32>()
                    / lengths.len() as f32;
                let std_dev = variance.sqrt();

                // Coefficient of variation: std_dev / mean
                // AI-generated text typically has CV < 0.25 (very uniform)
                // Human text typically has CV > 0.35
                let cv = if avg_len > 0.0 {
                    std_dev / avg_len
                } else {
                    1.0
                };

                // Very low variance is a strong AI signal
                if cv < 0.20 && lengths.len() >= 8 {
                    diagnostics.push(Diagnostic {
                        category: Category::Tone,
                        severity: Severity::Warning,
                        message: format!(
                            "Suspiciously uniform sentence lengths (CV={:.2}). AI-generated text typically has low variance.",
                            cv
                        ),
                        suggestion: Some(
                            "Vary your sentence lengths for more natural rhythm.".into(),
                        ),
                        location: anchor_location.clone(),
                        span: (anchor, (anchor + 100).min(text.len())),
                        snippet: "Document-level analysis".into(),
                    });
                    *counts.entry(Category::Tone).or_default() += 1;
                }
            }

            // Check for passive voice density
            let passive_re = &*PASSIVE_VOICE_RE;
            let passive_total = sentences
                .iter()
                .filter(|(_, off)| !filtered.is_category_disabled(*off, Category::Tone))
                .count();
            if passive_total > 0 {
                let passive_count = sentences
                    .iter()
                    .filter(|(s, off)| {
                        !filtered.is_category_disabled(*off, Category::Tone)
                            && passive_re.is_match(s)
                    })
                    .count();
                let passive_ratio = passive_count as f32 / passive_total as f32;

                // >50% passive voice is a strong AI signal
                if passive_ratio > 0.5 && passive_total >= 6 {
                    diagnostics.push(Diagnostic {
                        category: Category::Tone,
                        severity: Severity::Hint,
                        message: format!(
                            "High passive voice density ({:.0}% of sentences). Consider using active voice.",
                            passive_ratio * 100.0
                        ),
                        suggestion: Some(
                            "Rewrite passive constructions as active statements.".into(),
                        ),
                        location: anchor_location.clone(),
                        span: (anchor, (anchor + 100).min(text.len())),
                        snippet: "Document-level analysis".into(),
                    });
                    *counts.entry(Category::Tone).or_default() += 1;
                }
            }
        }

        if cadence_enabled {
            // Check for repeated sentence openings (first 2-3 words)
            let mut opening_counts: HashMap<String, usize> = HashMap::new();
            let mut opening_total = 0usize;
            for (sentence, off) in sentences {
                if filtered.is_category_disabled(*off, Category::Cadence) {
                    continue;
                }
                let trimmed = sentence.trim();
                if trimmed.starts_with('#') || trimmed.starts_with('-') || trimmed.starts_with('*')
                {
                    continue;
                }
                let words: Vec<&str> = trimmed.split_whitespace().take(3).collect();
                if words.len() >= 2 {
                    opening_total += 1;
                    let opening = words[..2.min(words.len())].join(" ").to_lowercase();
                    *opening_counts.entry(opening).or_default() += 1;
                }
            }

            // If any opening appears in >40% of sentences, that's suspicious
            if opening_total > 0 {
                for (opening, count) in opening_counts {
                    let ratio = count as f32 / opening_total as f32;
                    if ratio > 0.4 && count >= 4 {
                        diagnostics.push(Diagnostic {
                            category: Category::Cadence,
                            severity: Severity::Hint,
                            message: format!(
                                "Repetitive sentence opening `{}...` used in {:.0}% of sentences.",
                                opening,
                                ratio * 100.0
                            ),
                            suggestion: Some("Vary your sentence openings for better flow.".into()),
                            location: anchor_location.clone(),
                            span: (anchor, (anchor + 100).min(text.len())),
                            snippet: "Document-level analysis".into(),
                        });
                        *counts.entry(Category::Cadence).or_default() += 1;
                        break; // Only report once
                    }
                }
            }
        }
    }

    fn detect_min_code_blocks(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        profile: &ProfileRuntime,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        let Some(min_blocks) = profile.min_code_blocks else {
            return;
        };
        if min_blocks == 0 {
            return;
        }
        let mut blocks = 0usize;
        for line in text.lines() {
            if line.trim_start().starts_with("```") {
                blocks += 1;
            }
        }
        if blocks < min_blocks {
            let Some(anchor) = analysis_anchor_offset(filtered, text) else {
                return;
            };
            if filtered.is_category_disabled(anchor, Category::Structure) {
                return;
            }
            let location = byte_to_location(text, anchor);
            if filtered.is_line_ignored(location.line) {
                return;
            }
            diagnostics.push(Diagnostic {
                category: Category::Structure,
                severity: Severity::Warning,
                message: format!(
                    "Document has {} code block fences; minimum is {}.",
                    blocks, min_blocks
                ),
                suggestion: Some("Add runnable examples or configuration snippets.".into()),
                location,
                span: (anchor, anchor),
                snippet: String::new(),
            });
            *counts.entry(Category::Structure).or_default() += 1;
        }
    }

    fn detect_triad_slop(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        profile: &ProfileRuntime,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        if !profile.enable_triad_slop {
            return;
        }
        let triad: [&str; 3] = ["future development", "summary", "conclusion"];
        let mut present: Vec<HeadingCapture> = Vec::new();
        for (idx, line) in text.lines().enumerate() {
            if !line.trim_start().starts_with('#') {
                continue;
            }
            let content = line.trim_start_matches('#').trim().to_lowercase();
            if triad.iter().any(|t| content == *t) {
                let offset = line_offset(text, idx);
                if filtered.is_category_disabled(offset, Category::Structure) {
                    continue;
                }
                if filtered.is_line_ignored(idx + 1) {
                    continue;
                }
                present.push(HeadingCapture {
                    line: idx + 1,
                    column: 1,
                    offset,
                    len: line.len(),
                    text: line.to_string(),
                    lower: content,
                });
            }
        }
        if present.len() >= 2 {
            let cap = &present[0];
            if filtered.is_line_ignored(cap.line) {
                return;
            }
            diagnostics.push(Diagnostic {
                category: Category::Structure,
                severity: Severity::Warning,
                message: "Slop template triad detected (summary/conclusion/future development)."
                    .into(),
                suggestion: Some("Merge sections or remove boilerplate headings.".into()),
                location: Location {
                    line: cap.line,
                    column: cap.column,
                },
                span: (cap.offset, cap.offset + cap.len),
                snippet: cap.text.clone(),
            });
            *counts.entry(Category::Structure).or_default() += 1;
        }
    }

    fn detect_section_density(
        &self,
        text: &str,
        sentences: &[(String, usize)],
        filtered: &DisabledRanges,
        profile: &ProfileRuntime,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        let Some(min_sents) = profile.min_sentences_per_section else {
            return;
        };
        if min_sents == 0 {
            return;
        }
        // collect headings
        let mut heads: Vec<HeadingCapture> = Vec::new();
        for (idx, line) in text.lines().enumerate() {
            if !line.trim_start().starts_with('#') {
                continue;
            }
            let offset = line_offset(text, idx);
            if filtered.is_category_disabled(offset, Category::Structure) {
                continue;
            }
            let content = line.trim_start_matches('#').trim();
            if content.is_empty() {
                continue;
            }
            heads.push(HeadingCapture {
                line: idx + 1,
                column: 1,
                offset,
                len: line.len(),
                text: line.to_string(),
                lower: content.to_lowercase(),
            });
        }
        if heads.is_empty() {
            return;
        }
        // compute code fence ranges to ignore
        let mut fences: Vec<(usize, usize)> = Vec::new();
        let mut in_fence = false;
        let mut fence_start = 0usize;
        let mut cursor = 0usize;
        for seg in text.split_inclusive('\n') {
            let line = seg;
            if line.trim_start().starts_with("```") {
                if !in_fence {
                    in_fence = true;
                    fence_start = cursor;
                } else {
                    in_fence = false;
                    fences.push((fence_start, cursor + line.len()));
                }
            }
            cursor += line.len();
        }
        // evaluate each section
        for (i, cap) in heads.iter().enumerate() {
            let start = if let Some(rel) = text[cap.offset..].find('\n') {
                (cap.offset + rel + 1).min(text.len())
            } else {
                text.len()
            };
            let end = if i + 1 < heads.len() {
                heads[i + 1].offset
            } else {
                text.len()
            };
            if start >= end {
                continue;
            }
            let mut count = 0usize;
            for (sent, off) in sentences.iter() {
                if *off < start || *off >= end {
                    continue;
                }
                if filtered.is_category_disabled(*off, Category::Structure) {
                    continue;
                }
                // ignore bullets and empty
                let trimmed = sent.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if trimmed.starts_with('-') || trimmed.starts_with('*') {
                    continue;
                }
                // ignore sentences starting inside code fences
                if fences.iter().any(|(a, b)| *off >= *a && *off < *b) {
                    continue;
                }
                count += 1;
            }
            if count < min_sents {
                if filtered.is_line_ignored(cap.line) {
                    continue;
                }
                diagnostics.push(Diagnostic {
                    category: Category::Structure,
                    severity: Severity::Hint,
                    message: format!(
                        "Section `{}` is thin: {} sentences; minimum {}.",
                        cap.lower, count, min_sents
                    ),
                    suggestion: Some(
                        "Add concrete details, examples, or merge with adjacent sections.".into(),
                    ),
                    location: Location {
                        line: cap.line,
                        column: cap.column,
                    },
                    span: (cap.offset, end),
                    snippet: cap.text.clone(),
                });
                *counts.entry(Category::Structure).or_default() += 1;
            }
        }
    }
}

/// Precomputed disabled regions guarded by `<!-- dwg:off -->` ... `<!-- dwg:on -->` markers.
/// Precomputed disabled regions for the analyzer.
/// Supports:
/// - `<!-- dwg:off -->` ... `<!-- dwg:on -->` - disable all checks
/// - `<!-- dwg:ignore category -->` ... `<!-- dwg:end-ignore -->` - disable specific category
/// - `<!-- dwg:ignore-line -->` - disable the current line
/// - Code fences, inline code, URLs, frontmatter
struct DisabledRanges {
    /// Ranges where all checks are disabled
    global_ranges: Vec<(usize, usize)>,
    /// Ranges where specific categories are disabled
    category_ranges: HashMap<Category, Vec<(usize, usize)>>,
    /// Line numbers where all checks are disabled via ignore-line
    ignored_lines: HashSet<usize>,
}

/// Regex to match inline ignore comments: <!-- dwg:ignore category -->
static INLINE_IGNORE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"<!--\s*dwg:ignore\s+([a-z-,\s]+)\s*-->").expect("valid inline ignore regex")
});

/// Regex to match end-ignore comments: <!-- dwg:end-ignore -->
static END_IGNORE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"<!--\s*dwg:end-ignore\s*-->").expect("valid end ignore regex"));

/// Regex to match ignore-line comments: <!-- dwg:ignore-line -->
static IGNORE_LINE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"<!--\s*dwg:ignore-line\s*-->").expect("valid ignore line regex"));

impl DisabledRanges {
    fn new(text: &str) -> Self {
        let mut global_ranges = Vec::new();
        let mut category_ranges: HashMap<Category, Vec<(usize, usize)>> = HashMap::new();
        let mut ignored_lines: HashSet<usize> = HashSet::new();

        // Explicit dwg:off ranges (global disable).
        let mut cursor = 0;
        let bytes = text.as_bytes();
        while let Some(start_idx) = find_subsequence(bytes, b"<!-- dwg:off -->", cursor) {
            let search_from = start_idx + "<!-- dwg:off -->".len();
            if let Some(end_idx) = find_subsequence(bytes, b"<!-- dwg:on -->", search_from) {
                global_ranges.push((start_idx, end_idx + "<!-- dwg:on -->".len()));
                cursor = end_idx + "<!-- dwg:on -->".len();
            } else {
                global_ranges.push((start_idx, text.len()));
                break;
            }
        }

        // Category-specific ignores: <!-- dwg:ignore category --> ... <!-- dwg:end-ignore -->
        for cap in INLINE_IGNORE_RE.captures_iter(text) {
            let full_match = cap.get(0).unwrap();
            let categories = cap.get(1).unwrap().as_str();
            let start = full_match.start();

            // Find the end-ignore marker
            let search_from = full_match.end();
            if let Some(end_match) = END_IGNORE_RE.find(&text[search_from..]) {
                let end = search_from + end_match.end();
                let mut found_any = false;
                for raw in categories.split(',') {
                    let name = raw.trim();
                    if name.is_empty() {
                        continue;
                    }
                    if let Some(cat) = parse_category(name) {
                        category_ranges.entry(cat).or_default().push((start, end));
                        found_any = true;
                    }
                }
                if !found_any {
                    global_ranges.push((start, end));
                }
            } else {
                // If no end marker, apply to rest of document
                let mut found_any = false;
                for raw in categories.split(',') {
                    let name = raw.trim();
                    if name.is_empty() {
                        continue;
                    }
                    if let Some(cat) = parse_category(name) {
                        category_ranges
                            .entry(cat)
                            .or_default()
                            .push((start, text.len()));
                        found_any = true;
                    }
                }
                if !found_any {
                    global_ranges.push((start, text.len()));
                }
            }
        }

        // Ignore-line markers: <!-- dwg:ignore-line -->
        let mut line_num = 1usize;
        for line in text.lines() {
            if IGNORE_LINE_RE.is_match(line) {
                ignored_lines.insert(line_num);
                if line.trim() == "<!-- dwg:ignore-line -->" {
                    ignored_lines.insert(line_num + 1);
                }
            }
            line_num += 1;
        }

        // YAML frontmatter at the top of the file.
        if let Some(first_line) = text.lines().next() {
            if first_line.trim() == "---" {
                let mut end = 0usize;
                for line in text.split_inclusive('\n') {
                    end += line.len();
                    let trimmed = line.trim();
                    if trimmed == "---" || trimmed == "..." {
                        global_ranges.push((0, end));
                        break;
                    }
                }
            }
        }

        // Fenced code blocks (``` or ~~~).
        let mut in_fence = false;
        let mut fence_start = 0usize;
        let mut offset = 0usize;
        for line in text.split_inclusive('\n') {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                if !in_fence {
                    in_fence = true;
                    fence_start = offset;
                } else {
                    in_fence = false;
                    global_ranges.push((fence_start, offset + line.len()));
                }
            }
            offset += line.len();
        }
        if in_fence {
            global_ranges.push((fence_start, text.len()));
        }

        // Inline code spans on a single line.
        let bytes = text.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] == b'`' && !is_in_ranges(&global_ranges, i) {
                if bytes.get(i + 1) == Some(&b'`') && bytes.get(i + 2) == Some(&b'`') {
                    i += 3;
                    continue;
                }
                let mut j = i + 1;
                while j < bytes.len() && bytes[j] != b'\n' {
                    if bytes[j] == b'`' {
                        global_ranges.push((i, j + 1));
                        i = j + 1;
                        break;
                    }
                    j += 1;
                }
                if j >= bytes.len() || bytes[j] == b'\n' {
                    i += 1;
                }
            } else {
                i += 1;
            }
        }

        // Raw URLs.
        for mat in URL_RE.find_iter(text) {
            if is_in_ranges(&global_ranges, mat.start()) {
                continue;
            }
            global_ranges.push((mat.start(), mat.end()));
        }

        Self {
            global_ranges,
            category_ranges,
            ignored_lines,
        }
    }

    /// Check if a byte offset is disabled globally (all checks).
    fn is_disabled(&self, byte_offset: usize) -> bool {
        self.global_ranges
            .iter()
            .any(|(start, end)| byte_offset >= *start && byte_offset < *end)
    }

    /// Check if a specific category is disabled at the given byte offset.
    fn is_category_disabled(&self, byte_offset: usize, category: Category) -> bool {
        // First check global disables
        if self.is_disabled(byte_offset) {
            return true;
        }
        // Then check category-specific disables
        if let Some(ranges) = self.category_ranges.get(&category) {
            if ranges
                .iter()
                .any(|(start, end)| byte_offset >= *start && byte_offset < *end)
            {
                return true;
            }
        }
        false
    }

    /// Check if a specific line number is ignored via <!-- dwg:ignore-line -->.
    fn is_line_ignored(&self, line_num: usize) -> bool {
        self.ignored_lines.contains(&line_num)
    }
}

fn analysis_anchor_offset(filtered: &DisabledRanges, text: &str) -> Option<usize> {
    if text.is_empty() {
        return Some(0);
    }
    for (idx, _) in text.char_indices() {
        if !filtered.is_disabled(idx) {
            return Some(idx);
        }
    }
    None
}

fn find_subsequence(buf: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(start);
    }
    buf[start..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|pos| pos + start)
}

fn is_in_ranges(ranges: &[(usize, usize)], pos: usize) -> bool {
    ranges
        .iter()
        .any(|(start, end)| pos >= *start && pos < *end)
}

const EMOJI_HINTS: [char; 28] = [
    '😀', '😁', '😂', '🤣', '😃', '😄', '😅', '😊', '😍', '🤩', '🤔', '🚀', '🌟', '🔥', '✨', '💡',
    '✅', '❗', '⚡', '📈', '🎯', '📌', '👉', '⚠', '💥', '⭐', '🎉', '🧠',
];

const ALLOWED_SUFFIXES: [&str; 6] = ["s", "es", "ed", "ing", "ly", "d"];

fn is_emoji_hint(ch: char) -> bool {
    EMOJI_HINTS.contains(&ch)
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '-' | '\'')
}

fn has_word_boundary(text: &str, start: usize, end: usize) -> bool {
    let prev = text[..start].chars().rev().next();
    if let Some(ch) = prev {
        if is_word_char(ch) {
            return false;
        }
    }

    let rest = &text[end..];
    let next = rest.chars().next();
    let Some(next_ch) = next else {
        return true;
    };
    if !is_word_char(next_ch) {
        return true;
    }
    if !next_ch.is_ascii_alphabetic() {
        return false;
    }

    for suffix in ALLOWED_SUFFIXES {
        if let Some(prefix) = rest.get(..suffix.len()) {
            if prefix.eq_ignore_ascii_case(suffix) {
                let after = rest[suffix.len()..].chars().next();
                if after.map_or(true, |ch| !is_word_char(ch)) {
                    return true;
                }
            }
        }
    }

    false
}

fn find_term_with_boundary(haystack: &str, needle: &str) -> Option<usize> {
    let needle = needle.trim();
    if needle.is_empty() {
        return None;
    }
    let mut start = 0usize;
    while start < haystack.len() {
        let slice = &haystack[start..];
        let Some(rel) = slice.find(needle) else {
            return None;
        };
        let abs = start + rel;
        let end = abs + needle.len();
        if end <= haystack.len() && has_word_boundary(haystack, abs, end) {
            return Some(abs);
        }
        start = abs + 1;
    }
    None
}

fn matches_bold_list(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("- **") || trimmed.starts_with("* **")
}

fn appears_title_case(content: &str) -> bool {
    let words: Vec<&str> = content
        .split_whitespace()
        .filter(|w| !w.is_empty())
        .collect();
    if words.len() <= 1 {
        return false;
    }
    let mut title_case_words = 0;
    for &word in &words {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            if first.is_uppercase() && chars.all(|c| !c.is_uppercase()) {
                title_case_words += 1;
            }
        }
    }
    title_case_words >= 2
}

fn slice_snippet(text: &str, start: usize, end: usize) -> String {
    text.get(start..end).unwrap_or("").trim().to_string()
}

fn line_offset(text: &str, line_idx: usize) -> usize {
    let mut offset = 0;
    for (idx, line) in text.lines().enumerate() {
        if idx == line_idx {
            break;
        }
        offset += line.len() + 1; // include newline
    }
    offset
}

fn split_paragraphs_with_offset(text: &str) -> Vec<(&str, usize)> {
    if text.is_empty() {
        return vec![(text, 0)];
    }
    let mut result = Vec::new();
    let mut last = 0;
    for (idx, _) in text.match_indices("\n\n") {
        let para = &text[last..idx];
        result.push((para, last));
        last = idx + 2; // skip separator
    }
    if last <= text.len() {
        let para = &text[last..];
        result.push((para, last));
    }
    if result.is_empty() {
        result.push((text, 0));
    }
    result
}

fn split_sentences_with_offset(text: &str) -> Vec<(String, usize)> {
    let mut sentences = Vec::new();
    let mut sentence = String::new();
    let mut start_byte = 0;
    let mut active = false;
    let mut chars = text.char_indices().peekable();

    while let Some((idx, ch)) = chars.next() {
        if !active {
            start_byte = idx;
            active = true;
        }
        sentence.push(ch);

        let mut should_flush = false;

        if matches!(ch, '.' | '!' | '?') {
            should_flush = match chars.peek() {
                Some((_, next_ch)) => next_ch.is_whitespace(),
                None => true,
            };
        }

        if ch == '\n' {
            let trimmed_current = sentence.trim_start();
            if trimmed_current.starts_with('#')
                || trimmed_current.starts_with("- ")
                || trimmed_current.starts_with("* ")
                || is_numbered_list(trimmed_current)
            {
                should_flush = true;
            } else {
                let mut temp = chars.clone();
                let mut saw_blank_line = false;
                let mut next_non_ws = None;
                while let Some(&(next_idx, next_ch)) = temp.peek() {
                    if next_ch == '\r' {
                        temp.next();
                        continue;
                    }
                    if next_ch == '\n' {
                        saw_blank_line = true;
                        temp.next();
                        continue;
                    }
                    if next_ch.is_whitespace() {
                        temp.next();
                        continue;
                    }
                    next_non_ws = Some((next_idx, next_ch));
                    break;
                }
                if saw_blank_line {
                    should_flush = true;
                } else if let Some((next_idx, _)) = next_non_ws {
                    let rest = &text[next_idx..];
                    let rest_line = rest.lines().next().unwrap_or("").trim_start();
                    if rest_line.starts_with('#')
                        || rest_line.starts_with("- ")
                        || rest_line.starts_with("* ")
                        || is_numbered_list(rest_line)
                    {
                        should_flush = true;
                    }
                } else {
                    should_flush = true;
                }
            }
        }

        if should_flush {
            let trimmed = sentence.trim();
            if !trimmed.is_empty() {
                sentences.push((trimmed.to_string(), start_byte));
            }
            sentence.clear();
            active = false;
        }
    }

    if !sentence.trim().is_empty() {
        sentences.push((sentence.trim().to_string(), start_byte));
    }

    sentences
}

fn sentence_index_for_offset(sentences: &[(String, usize)], offset: usize) -> Option<usize> {
    for (idx, (sentence, start)) in sentences.iter().enumerate() {
        if offset >= *start && offset < *start + sentence.len() {
            return Some(idx);
        }
    }
    None
}

fn sentence_has_specifics(sentence: &str) -> bool {
    SPECIFICITY_RE.is_match(sentence)
}

fn sentence_has_citation(sentence: &str) -> bool {
    CITATION_RE.is_match(sentence)
}

fn percent_claim_is_contextual(sentence: &str) -> bool {
    PERCENT_CONTEXT_OK_RE.is_match(sentence)
}

fn normalize_sentence(sentence: &str) -> String {
    let mut normalised = String::with_capacity(sentence.len());
    let mut last_was_space = false;
    for ch in sentence.chars() {
        if ch.is_alphanumeric() {
            normalised.push(ch.to_ascii_lowercase());
            last_was_space = false;
        } else if ch.is_whitespace() {
            if !last_was_space {
                normalised.push(' ');
                last_was_space = true;
            }
        }
    }
    normalised.trim().to_string()
}

fn is_numbered_list(line: &str) -> bool {
    let mut chars = line.chars().peekable();
    let mut saw_digit = false;
    while let Some(ch) = chars.next() {
        if ch.is_ascii_digit() {
            saw_digit = true;
            continue;
        }
        if ch == '.' || ch == ')' {
            return saw_digit;
        }
        if ch.is_whitespace() {
            if saw_digit {
                continue;
            } else {
                return false;
            }
        }
        return false;
    }
    false
}

static CONFIDENCE_PERCENT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b\d{2,}%").expect("valid percent regex"));

static MID_SENTENCE_QUESTION_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\?\s+[a-z]").expect("valid question regex"));

/// Detect passive voice constructions.
/// Matches patterns like "is/was/were/been/being + past participle"
static PASSIVE_VOICE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?ix)
        \b(?:is|are|was|were|be|been|being)\s+
        (?:
            \w+ed\b |           # Regular past participles (created, handled)
            \w+en\b |           # Irregular (written, taken, given)
            made\b | done\b | said\b | seen\b | known\b | shown\b |
            built\b | sent\b | left\b | found\b | told\b | thought\b |
            used\b | called\b | considered\b | designed\b | intended\b
        )
        ",
    )
    .expect("valid passive voice regex")
});

static BOLD_SPAN_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)(\*\*[^*]+\*\*|__[^_]+__)").expect("valid bold span regex"));

static SPECIFICITY_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?x)
        # Semantic versions and simple versions
        \bv\d+(?:\.\d+)*\b |                   # v1, v2, v1.0, v1.2.3
        \b\d+\.\d+(?:\.\d+)+\b |               # 1.2.3 (at least 2 dots)
        
        # Issue/ticket references
        \#\d{2,}\b |                           # GitHub issues (#123)
        \b[A-Z]{2,}-\d+\b |                    # Jira tickets (PROJ-123)
        
        # Code identifiers (case-sensitive)
        \b[a-z]{2,}[A-Z][A-Za-z0-9]{2,}\b |    # camelCase
        \b[a-z]{2,}_[a-z0-9_]{2,}\b |          # snake_case
        \b[A-Z]{2}[A-Z0-9_]{2,}\b |            # CONSTANTS (min 4 uppercase)
        
        # File paths with slash
        /[\w./-]+ |
        
        # URLs
        https?://\S+ |
        www\.\S+ |
        
        # Figure/Table/Listing references (case insensitive)
        (?i)\b(?:Figure|Fig|Table|Tbl|Listing|Example|Appendix)\s+\d+(?-i) |
        
        # Technical references
        (?i)\b(?:RFC|ISO|IEEE)\s*\d+(?-i) |
        \bport\s+\d{2,5}\b |
        \$[A-Z][A-Z0-9_]+ |
        
        # Function/method calls
        \b[a-zA-Z_]\w*\(\) |
        \b\w+::\w+ |
        
        # Quantities with units (case insensitive for units)
        \b\d+\s*(?i)(?:ms|sec|min|hours?|KB|MB|GB|TB|bytes?)(?-i)\b |
        \b\d{2,}%\s+of\b
    ",
    )
    .expect("valid specificity regex")
});

static URL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\bhttps?://[^\s<>()]+|\bwww\.[^\s<>()]+").expect("valid url regex")
});

static CITATION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?ix)
        \[[0-9]{1,3}\] |
        \[[^\]]+\]\([^)]+\) |
        \([A-Z][A-Za-z]+(?:\s+et\ al\.)?,?\s+\d{4}\) |
        \bdoi:\S+ |
        \bhttps?://\S+ |
        \bwww\.\S+
    ",
    )
    .expect("valid citation regex")
});

static PERCENT_CONTEXT_OK_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?ix)
        \b\d{2,}%\s+
        (?:
            of\b |
            (?:test\s+)?coverage\b |
            uptime\b |
            availability\b |
            requests?\b |
            traffic\b |
            samples?\b |
            pass(?:\s+rate)?\b |
            success(?:\s+rate)?\b |
            failure(?:\s+rate)?\b |
            errors?\b
        )
    ",
    )
    .expect("valid percent context regex")
});

fn count_words(text: &str) -> usize {
    text.split_whitespace()
        .filter(|w| w.chars().any(|c| c.is_alphabetic()))
        .count()
}

fn byte_to_location(text: &str, byte_offset: usize) -> Location {
    let mut line = 1;
    let mut last_newline = 0;
    for (idx, ch) in text.char_indices() {
        if idx >= byte_offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            last_newline = idx + 1;
        }
    }
    let column = text[last_newline..byte_offset].chars().count() + 1;
    Location { line, column }
}

fn replacement_for(phrase: &str) -> Option<String> {
    let mut map = HashMap::new();
    map.insert("delve into", "look at");
    map.insert("navigate the landscape", "map the area");
    map.insert("delve", "look at");
    map.insert("deep dive", "look closely");
    map.insert("underscores", "shows");
    map.insert("showcasing", "showing");
    map.insert("pivotal", "important");
    map.insert("realm", "field");
    map.insert("meticulous", "detailed");
    map.insert("leverage", "use");
    map.insert("utilise", "use");
    map.insert("utilize", "use");
    map.insert("facilitate", "help");
    map.insert("optimise", "improve");
    map.insert("optimize", "improve");
    map.insert("embark", "start");
    map.insert("embark on a journey", "start");
    map.insert("underscore", "highlight");
    map.insert("aims to explore", "studies");
    map.insert("aligns", "fits");
    map.insert("seamless", "smooth");
    map.insert("seamlessly", "smoothly");
    map.insert("robust", "solid");
    map.insert("robustly", "solidly");
    map.insert("innovative", "new");
    map.insert("transformative", "changing");
    map.insert("unprecedented", "new");
    map.insert("plethora", "many");
    map.insert("empower", "help");
    map.get(phrase).map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analyzer() -> Analyzer {
        Analyzer::new(Config::default()).unwrap()
    }

    fn analyze_default(analyzer: &Analyzer, text: &str) -> DocumentReport {
        let profile = analyzer.profile_for_name("default").unwrap();
        analyzer.analyze_with_profile(text, profile)
    }

    #[test]
    fn detects_puffery() {
        let a = analyzer();
        let report = analyze_default(&a, "This update stands as a testament to progress.");
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(report.diagnostics[0].category, Category::Puffery);
    }

    #[test]
    fn detects_buzzword() {
        let a = analyzer();
        let report = analyze_default(&a, "We will delve into the details tomorrow.");
        assert_eq!(report.diagnostics[0].category, Category::Buzzword);
    }

    #[test]
    fn detects_negative_parallelism() {
        let a = analyzer();
        let report = analyze_default(&a, "It is not just speed but also quality that matters.");
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.category == Category::NegativeParallel));
    }

    #[test]
    fn detects_connector_glut() {
        let a = analyzer();
        let report = analyze_default(
            &a,
            "However, we launched, and furthermore we iterated, while we simultaneously refined.",
        );
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.category == Category::ConnectorGlut));
    }

    #[test]
    fn respects_whitelist_phrase() {
        let mut cfg = Config::default();
        cfg.whitelist.allowed_phrases.push("just ship it".into());
        let a = Analyzer::new(cfg).unwrap();
        let profile = a.profile_for_name("default").unwrap();
        let report =
            a.analyze_with_profile("we wrap up the change and just ship it tomorrow.", profile);
        assert!(report.diagnostics.is_empty());
    }

    #[test]
    fn detects_weasel_range() {
        let a = analyzer();
        let report = analyze_default(
            &a,
            "This covers everything from onboarding to retention to advocacy.",
        );
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.category == Category::Weasel));
    }

    #[test]
    fn allows_weasel_with_citation() {
        let a = analyzer();
        let report = analyze_default(&a, "Experts say the change improved results [1].");
        assert!(report
            .diagnostics
            .iter()
            .all(|d| d.category != Category::Weasel));
    }

    #[test]
    fn detects_transition_phrase() {
        let a = analyzer();
        let report = analyze_default(&a, "Furthermore, we will ship the feature tomorrow.");
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.category == Category::Transition));
    }

    #[test]
    fn detects_marketing_cliche() {
        let a = analyzer();
        let report = analyze_default(
            &a,
            "This is a game-changing solution that unlocks the power of data.",
        );
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.category == Category::Marketing));
    }

    #[test]
    fn splits_sentences_around_bullets() {
        let a = analyzer();
        let text = "Intro sentence. Another sentence.\n\n- bullet one\n- bullet two\n\nConclusion sentence.";
        let report = analyze_default(&a, text);
        assert!(!report
            .diagnostics
            .iter()
            .any(|d| d.category == Category::SentenceLength));
    }

    #[test]
    fn detects_broad_terms() {
        let mut cfg = Config::default();
        cfg.profile_defaults.broad_terms = vec!["solution".into()];
        let a = Analyzer::new(cfg).unwrap();
        let profile = a.profile_for_name("default").unwrap();
        let report = a.analyze_with_profile("This solution will do everything.", profile);
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.category == Category::BroadTerm));
    }

    #[test]
    fn detects_confidence_claims() {
        let mut cfg = Config::default();
        cfg.profile_defaults.confidence_phrases = vec!["industry-leading".into()];
        let a = Analyzer::new(cfg).unwrap();
        let profile = a.profile_for_name("default").unwrap();
        let report = a.analyze_with_profile(
            "Our industry-leading tool hits 95% accuracy every time.",
            profile,
        );
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.category == Category::Confidence));
    }

    #[test]
    fn allows_confidence_with_citation() {
        let mut cfg = Config::default();
        cfg.profile_defaults.confidence_phrases = vec!["industry-leading".into()];
        let a = Analyzer::new(cfg).unwrap();
        let profile = a.profile_for_name("default").unwrap();
        let report = a.analyze_with_profile(
            "Our industry-leading tool leads benchmarks (Smith, 2024).",
            profile,
        );
        assert!(report
            .diagnostics
            .iter()
            .all(|d| d.category != Category::Confidence));
    }

    #[test]
    fn ignores_code_fences_and_inline_code() {
        let a = analyzer();
        let text = "Intro sentence.\n\n```\nWe will delve into the code here.\n```\n\nUse `utilize` in a snippet.\n\nWe will delve into details.";
        let report = analyze_default(&a, text);
        let buzzwords: Vec<&Diagnostic> = report
            .diagnostics
            .iter()
            .filter(|d| d.category == Category::Buzzword)
            .collect();
        assert_eq!(buzzwords.len(), 1);
    }

    #[test]
    fn suppresses_single_buzzword_with_specifics() {
        let a = analyzer();
        let report = analyze_default(&a, "We used a robust API v2 for the rollout.");
        assert!(report
            .diagnostics
            .iter()
            .all(|d| d.category != Category::Buzzword));
    }

    #[test]
    fn flags_buzzword_cluster_with_specifics() {
        let a = analyzer();
        let report = analyze_default(&a, "We used a robust, seamless API v2 to ship the update.");
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.category == Category::Buzzword));
    }

    #[test]
    fn detects_emoji_bullet() {
        let a = analyzer();
        let report = analyze_default(&a, "- ✅ Ship the change\n- plain follow-up");
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.category == Category::Formatting));
    }

    #[test]
    fn detects_mid_sentence_question() {
        let a = analyzer();
        let report = analyze_default(&a, "This seems odd? it keeps going anyway.");
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.category == Category::Tone));
    }

    #[test]
    fn matches_common_suffix_buzzwords() {
        let a = analyzer();
        let report = analyze_default(&a, "We utilized the system.");
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.category == Category::Buzzword));
    }
}
