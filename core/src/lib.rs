//! Deterministic Writing Guard core analysis engine.
//! Implements deterministic rules that flag AI-styled prose based on
//! configurable phrase lists and structural heuristics.

use std::collections::{BTreeMap, HashMap, HashSet};

use aho_corasick::{AhoCorasick, AhoCorasickBuilder};
use regex::Regex;
use serde::{Deserialize, Serialize};

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
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            em_dashes_per_paragraph: 1,
            connectors_per_sentence: 1,
            rule_of_three_per_paragraph: 0,
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
            ],
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
                ],
            },
        }
    }
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
            Category::EmDash => "em-dash",
            Category::Formatting => "formatting",
            Category::QuoteStyle => "quote-style",
        };
        f.write_str(name)
    }
}

/// Location metadata in 1-based line/column coordinates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub line: usize,
    pub column: usize,
}

/// Style diagnostic emitted by the analyzer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub category: Category,
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
    template_regexes: Vec<Regex>,
    rule_of_three_regex: Regex,
    range_regex: Regex,
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

        let mut template_regexes = Vec::new();
        for pattern in &config.templates.ban {
            let pattern = pattern.trim();
            if pattern.is_empty() {
                continue;
            }
            let regex = Regex::new(&format!("(?i){pattern}"))
                .map_err(|e| anyhow::anyhow!("invalid template regex `{pattern}`: {e}"))?;
            template_regexes.push(regex);
        }

        let rule_of_three_regex =
            Regex::new(r"(?i)\b[\w-]+,\s+[\w-]+,\s+(?:and|&)\s+[\w-]+").expect("static regex");

        let range_regex =
            Regex::new(r"(?i)from [^\n,.;]+ to [^\n,.;]+ to [^\n,.;]+").expect("static regex");

        Ok(Self {
            config,
            allow_phrase_set,
            puffery_matcher,
            buzzword_matcher,
            weasel_matcher,
            transition_matcher,
            marketing_matcher,
            template_regexes,
            rule_of_three_regex,
            range_regex,
        })
    }

    /// Run analysis on input text and return all diagnostics.
    pub fn analyze(&self, text: &str) -> DocumentReport {
        let filtered = DisabledRanges::new(text);
        let mut diagnostics = Vec::new();
        let mut category_counts: BTreeMap<Category, usize> = BTreeMap::new();

        self.detect_puffery(text, &filtered, &mut diagnostics, &mut category_counts);
        self.detect_buzzwords(text, &filtered, &mut diagnostics, &mut category_counts);
        self.detect_transitions(text, &filtered, &mut diagnostics, &mut category_counts);
        self.detect_marketing(text, &filtered, &mut diagnostics, &mut category_counts);
        self.detect_templates(text, &filtered, &mut diagnostics, &mut category_counts);
        self.detect_ranges(text, &filtered, &mut diagnostics, &mut category_counts);
        self.detect_connectors(text, &filtered, &mut diagnostics, &mut category_counts);
        self.detect_rule_of_three(text, &filtered, &mut diagnostics, &mut category_counts);
        self.detect_em_dash(text, &filtered, &mut diagnostics, &mut category_counts);
        self.detect_headings(text, &filtered, &mut diagnostics, &mut category_counts);
        self.detect_quotes(text, &filtered, &mut diagnostics, &mut category_counts);

        let word_count = count_words(text);

        DocumentReport {
            word_count,
            diagnostics,
            category_counts,
        }
    }

    fn detect_puffery(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        if let Some(matcher) = &self.puffery_matcher {
            for mat in matcher.find_iter(text.as_bytes()) {
                if filtered.is_disabled(mat.start()) {
                    continue;
                }
                let snippet = slice_snippet(text, mat.start(), mat.end());
                if self.allow_phrase_set.contains(&snippet.to_lowercase()) {
                    continue;
                }
                let location = byte_to_location(text, mat.start());
                diagnostics.push(Diagnostic {
                    category: Category::Puffery,
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
                if filtered.is_disabled(mat.start()) {
                    continue;
                }
                let snippet = slice_snippet(text, mat.start(), mat.end());
                if self.allow_phrase_set.contains(&snippet.to_lowercase()) {
                    continue;
                }
                let location = byte_to_location(text, mat.start());
                diagnostics.push(Diagnostic {
                    category: Category::Weasel,
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
        filtered: &DisabledRanges,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        if let Some(matcher) = &self.buzzword_matcher {
            for mat in matcher.find_iter(text.as_bytes()) {
                if filtered.is_disabled(mat.start()) {
                    continue;
                }
                let snippet = slice_snippet(text, mat.start(), mat.end());
                if self.allow_phrase_set.contains(&snippet.to_lowercase()) {
                    continue;
                }
                let suggestion = replacement_for(&snippet.to_lowercase());
                let location = byte_to_location(text, mat.start());
                diagnostics.push(Diagnostic {
                    category: Category::Buzzword,
                    message: format!("Buzzword detected: `{snippet}`"),
                    suggestion,
                    location,
                    span: (mat.start(), mat.end()),
                    snippet,
                });
                *counts.entry(Category::Buzzword).or_default() += 1;
            }
        }
    }

    fn detect_transitions(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        if let Some(matcher) = &self.transition_matcher {
            for mat in matcher.find_iter(text.as_bytes()) {
                if filtered.is_disabled(mat.start()) {
                    continue;
                }
                let snippet = slice_snippet(text, mat.start(), mat.end());
                if self.allow_phrase_set.contains(&snippet.to_lowercase()) {
                    continue;
                }
                let location = byte_to_location(text, mat.start());
                diagnostics.push(Diagnostic {
                    category: Category::Transition,
                    message: format!("Transitional filler detected: `{snippet}`"),
                    suggestion: Some("Trim or replace with a simple connector.".into()),
                    location,
                    span: (mat.start(), mat.end()),
                    snippet,
                });
                *counts.entry(Category::Transition).or_default() += 1;
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
                if filtered.is_disabled(mat.start()) {
                    continue;
                }
                let snippet = slice_snippet(text, mat.start(), mat.end());
                if self.allow_phrase_set.contains(&snippet.to_lowercase()) {
                    continue;
                }
                let location = byte_to_location(text, mat.start());
                diagnostics.push(Diagnostic {
                    category: Category::Marketing,
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
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        for regex in &self.template_regexes {
            for mat in regex.find_iter(text) {
                if filtered.is_disabled(mat.start()) {
                    continue;
                }
                let snippet = slice_snippet(text, mat.start(), mat.end());
                if self.allow_phrase_set.contains(&snippet.to_lowercase()) {
                    continue;
                }
                let cat = if snippet.to_lowercase().contains("not") {
                    Category::NegativeParallel
                } else {
                    Category::Template
                };
                let location = byte_to_location(text, mat.start());
                diagnostics.push(Diagnostic {
                    category: cat,
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
        filtered: &DisabledRanges,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        let connectors = [
            "while",
            "although",
            "however",
            "furthermore",
            "simultaneously",
            "nevertheless",
            "at the same time",
            "moreover",
        ];

        for (sentence, offset) in split_sentences_with_offset(text) {
            if filtered.is_disabled(offset) {
                continue;
            }
            let lower = sentence.to_lowercase();
            let mut count = 0;
            for connector in connectors.iter() {
                count += lower.matches(connector).count();
            }
            if count > self.config.limits.connectors_per_sentence {
                let location = byte_to_location(text, offset);
                diagnostics.push(Diagnostic {
                    category: Category::ConnectorGlut,
                    message: format!(
                        "Sentence uses {} connectors; limit is {}.",
                        count, self.config.limits.connectors_per_sentence
                    ),
                    suggestion: Some("Split the sentence or drop extra connectors.".into()),
                    location,
                    span: (offset, offset + sentence.len()),
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
            if filtered.is_disabled(mat.start()) {
                continue;
            }
            let snippet = slice_snippet(text, mat.start(), mat.end());
            let location = byte_to_location(text, mat.start());
            diagnostics.push(Diagnostic {
                category: Category::Weasel,
                message: format!("Exaggerated range detected: `{snippet}`"),
                suggestion: Some("List the specific items or tighten the range.".into()),
                location,
                span: (mat.start(), mat.end()),
                snippet,
            });
            *counts.entry(Category::Weasel).or_default() += 1;
        }
    }

    fn detect_rule_of_three(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        for (paragraph, offset) in split_paragraphs_with_offset(text) {
            if filtered.is_disabled(offset) {
                continue;
            }
            if paragraph.trim().is_empty() {
                continue;
            }
            let mut seen = 0;
            for mat in self.rule_of_three_regex.find_iter(paragraph) {
                let m_start = offset + mat.start();
                if filtered.is_disabled(m_start) {
                    continue;
                }
                seen += 1;
                if seen > self.config.limits.rule_of_three_per_paragraph {
                    let snippet = slice_snippet(text, m_start, m_start + mat.as_str().len());
                    let location = byte_to_location(text, m_start);
                    diagnostics.push(Diagnostic {
                        category: Category::RuleOfThree,
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
            if filtered.is_disabled(offset) {
                continue;
            }
            if paragraph.trim().is_empty() {
                continue;
            }
            let occurrences = paragraph.matches('—').count();
            if occurrences > self.config.limits.em_dashes_per_paragraph {
                let location = byte_to_location(text, offset);
                diagnostics.push(Diagnostic {
                    category: Category::EmDash,
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

    fn detect_headings(
        &self,
        text: &str,
        filtered: &DisabledRanges,
        diagnostics: &mut Vec<Diagnostic>,
        counts: &mut BTreeMap<Category, usize>,
    ) {
        let emoji_chars: HashSet<char> = [
            '😀', '😁', '😂', '🤣', '😃', '😄', '😅', '😊', '😍', '🤩', '🤔', '🚀', '🌟', '🔥',
            '✨', '💡', '✅', '❗', '⚡', '📈', '🎯',
        ]
        .into_iter()
        .collect();

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
            let mut has_emoji = false;
            for ch in content.chars() {
                if emoji_chars.contains(&ch) {
                    has_emoji = true;
                    break;
                }
            }
            if has_emoji {
                diagnostics.push(Diagnostic {
                    category: Category::Formatting,
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

            if matches_bold_list(line) {
                diagnostics.push(Diagnostic {
                    category: Category::Formatting,
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
            {
                diagnostics.push(Diagnostic {
                    category: Category::Formatting,
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
            if filtered.is_disabled(idx) {
                continue;
            }
            if curly_chars.contains(&ch) {
                let location = byte_to_location(text, idx);
                diagnostics.push(Diagnostic {
                    category: Category::QuoteStyle,
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
}

/// Precomputed disabled regions guarded by `<!-- dwg:off -->` ... `<!-- dwg:on -->` markers.
struct DisabledRanges {
    ranges: Vec<(usize, usize)>,
}

impl DisabledRanges {
    fn new(text: &str) -> Self {
        let mut ranges = Vec::new();
        let mut cursor = 0;
        let bytes = text.as_bytes();
        while let Some(start_idx) = find_subsequence(bytes, b"<!-- dwg:off -->", cursor) {
            let search_from = start_idx + "<!-- dwg:off -->".len();
            if let Some(end_idx) = find_subsequence(bytes, b"<!-- dwg:on -->", search_from) {
                ranges.push((start_idx, end_idx + "<!-- dwg:on -->".len()));
                cursor = end_idx + "<!-- dwg:on -->".len();
            } else {
                ranges.push((start_idx, text.len()));
                break;
            }
        }
        Self { ranges }
    }

    fn is_disabled(&self, byte_offset: usize) -> bool {
        self.ranges
            .iter()
            .any(|(start, end)| byte_offset >= *start && byte_offset < *end)
    }
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
    for (idx, ch) in text.char_indices() {
        if sentence.is_empty() {
            start_byte = idx;
        }
        sentence.push(ch);
        if ['.', '!', '?'].contains(&ch) {
            sentences.push((sentence.clone(), start_byte));
            sentence.clear();
        }
    }
    if !sentence.trim().is_empty() {
        sentences.push((sentence, start_byte));
    }
    sentences
}

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

    #[test]
    fn detects_puffery() {
        let a = analyzer();
        let report = a.analyze("This update stands as a testament to progress.");
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(report.diagnostics[0].category, Category::Puffery);
    }

    #[test]
    fn detects_buzzword() {
        let a = analyzer();
        let report = a.analyze("We will delve into the details tomorrow.");
        assert_eq!(report.diagnostics[0].category, Category::Buzzword);
    }

    #[test]
    fn detects_negative_parallelism() {
        let a = analyzer();
        let report = a.analyze("It is not just speed but also quality that matters.");
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.category == Category::NegativeParallel));
    }

    #[test]
    fn detects_connector_glut() {
        let a = analyzer();
        let report = a.analyze(
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
        let report = a.analyze("we wrap up the change and just ship it tomorrow.");
        assert!(report.diagnostics.is_empty());
    }

    #[test]
    fn detects_weasel_range() {
        let a = analyzer();
        let report = a.analyze("This covers everything from onboarding to retention to advocacy.");
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.category == Category::Weasel));
    }

    #[test]
    fn detects_transition_phrase() {
        let a = analyzer();
        let report = a.analyze("Furthermore, we will ship the feature tomorrow.");
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.category == Category::Transition));
    }

    #[test]
    fn detects_marketing_cliche() {
        let a = analyzer();
        let report = a.analyze("This is a game-changing solution that unlocks the power of data.");
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.category == Category::Marketing));
    }
}
