use dwg_core::{Analyzer, Category, Config, DocumentReport};

fn analyze_with(cfg: Config, text: &str) -> DocumentReport {
    let analyzer = Analyzer::new(cfg).unwrap();
    analyzer.analyze(text)
}

fn analyze(text: &str) -> DocumentReport {
    analyze_with(Config::default(), text)
}

fn assert_has(report: &DocumentReport, category: Category) {
    assert!(
        report.diagnostics.iter().any(|d| d.category == category),
        "expected category {category:?}, got diagnostics: {:#?}",
        report.diagnostics
    );
}

fn assert_not(report: &DocumentReport, category: Category) {
    assert!(
        report.diagnostics.iter().all(|d| d.category != category),
        "expected no category {category:?}, got diagnostics: {:#?}",
        report.diagnostics
    );
}

#[test]
fn ignores_yaml_frontmatter() {
    let text = r#"---
title: "In conclusion"
notes: "We will leverage the system"
---

We will leverage the system."#;
    let report = analyze(text);
    assert_has(&report, Category::Buzzword);
    assert_not(&report, Category::Template);
}

#[test]
fn ignores_urls_even_if_they_contain_slop_terms() {
    let report = analyze("See https://example.com/deep-dive for details. We ship tomorrow.");
    assert_not(&report, Category::Buzzword);
}

#[test]
fn ignores_markdown_links_with_urls() {
    let report = analyze("See [notes](https://example.com/deep-dive). We ship tomorrow.");
    assert_not(&report, Category::Buzzword);
}

#[test]
fn ignores_code_fences_and_inline_code_for_templates() {
    let text = r#"
```text
As an AI language model, I can't access that.
```

This is fine. `As an AI language model` is in code."#;
    let report = analyze(text);
    assert_not(&report, Category::Template);
}

#[test]
fn detects_ai_assistant_disclaimer_template() {
    let report = analyze("As an AI language model, I cannot access external links.");
    assert_has(&report, Category::Template);
}

#[test]
fn detects_subject_line_template() {
    let report = analyze("Subject: Quick update\n\nWe shipped the fix.");
    assert_has(&report, Category::Template);
}

#[test]
fn suppresses_single_buzzword_when_sentence_has_specifics() {
    let report = analyze("We used a robust API v2 for RFC-1234.");
    assert_not(&report, Category::Buzzword);
}

#[test]
fn flags_buzzword_cluster_even_with_specifics() {
    let report = analyze("We used a robust, scalable API v2 for RFC-1234.");
    assert_has(&report, Category::Buzzword);
}

#[test]
fn suppresses_single_transition_when_sentence_has_specifics() {
    let report = analyze("Additionally, we shipped API v2.");
    assert_not(&report, Category::Transition);
}

#[test]
fn flags_transition_cluster_even_with_specifics() {
    let report = analyze("Additionally, in other words, we shipped API v2.");
    assert_has(&report, Category::Transition);
}

#[test]
fn broad_term_requires_word_boundary() {
    let mut cfg = Config::default();
    cfg.profile_defaults.broad_terms = vec!["solution".into(), "mission".into()];
    let report = analyze_with(cfg, "The resolution passed. The permission check failed.");
    assert_not(&report, Category::BroadTerm);
}

#[test]
fn broad_term_flags_generic_sentences() {
    let mut cfg = Config::default();
    cfg.profile_defaults.broad_terms = vec!["solution".into()];
    let report = analyze_with(cfg, "This solution will improve everything.");
    assert_has(&report, Category::BroadTerm);
}

#[test]
fn weasel_is_suppressed_when_citation_present() {
    let report = analyze("Experts say the change improved results [1].");
    assert_not(&report, Category::Weasel);
}

#[test]
fn confidence_is_suppressed_when_citation_present() {
    let mut cfg = Config::default();
    cfg.profile_defaults.confidence_phrases = vec!["world-class".into()];
    let report = analyze_with(cfg, "Our world-class tool leads benchmarks (Smith, 2024).");
    assert_not(&report, Category::Confidence);
}

#[test]
fn confidence_is_flagged_without_citation() {
    let mut cfg = Config::default();
    cfg.profile_defaults.confidence_phrases = vec!["world-class".into()];
    let report = analyze_with(cfg, "Our world-class tool leads benchmarks.");
    assert_has(&report, Category::Confidence);
}

#[test]
fn percent_claim_is_not_flagged_when_contextualized() {
    let report = analyze("95% of requests succeeded in the last window.");
    assert_not(&report, Category::Confidence);
}

#[test]
fn percent_claim_is_flagged_when_it_reads_like_marketing() {
    let report = analyze("Our model hits 95% accuracy every time.");
    assert_has(&report, Category::Confidence);
}

#[test]
fn percent_claim_is_not_flagged_for_test_coverage() {
    let report = analyze("We hit 95% test coverage.");
    assert_not(&report, Category::Confidence);
}

#[test]
fn bold_span_overuse_is_flagged() {
    let report = analyze("**One** **two** **three** **four** in one paragraph.");
    assert_has(&report, Category::Formatting);
}

#[test]
fn bold_lead_bullets_are_flagged_when_repeated() {
    let report = analyze("- **One:** a\n- **Two:** b\n- **Three:** c\n");
    assert_has(&report, Category::Formatting);
}

#[test]
fn bold_lead_bullets_are_not_flagged_when_rare() {
    let report = analyze("- **One:** a\n- Two: b\n- **Three:** c\n");
    assert_not(&report, Category::Formatting);
}

#[test]
fn emoji_bullet_is_flagged() {
    let report = analyze("- ✅ Ship the fix\n- follow up");
    assert_has(&report, Category::Formatting);
}

#[test]
fn mid_sentence_question_is_flagged() {
    let report = analyze("This feels odd? it keeps going anyway.");
    assert_has(&report, Category::Tone);
}

#[test]
fn normal_question_is_not_mid_sentence_question() {
    let report = analyze("Is this odd?");
    assert_not(&report, Category::Tone);
}

#[test]
fn ignore_line_on_own_line_ignores_next_line_too() {
    let text = r#"<!-- dwg:ignore-line -->
As an AI language model, I cannot access external links.
As an AI language model, I cannot access external links."#;
    let report = analyze(text);
    let templates = report
        .diagnostics
        .iter()
        .filter(|d| d.category == Category::Template)
        .count();
    assert_eq!(templates, 1, "expected exactly one template diagnostic");
}

#[test]
fn ignore_line_inline_only_ignores_that_line() {
    let text = r#"As an AI language model, I cannot access external links. <!-- dwg:ignore-line -->
As an AI language model, I cannot access external links."#;
    let report = analyze(text);
    let templates = report
        .diagnostics
        .iter()
        .filter(|d| d.category == Category::Template)
        .count();
    assert_eq!(templates, 1, "expected exactly one template diagnostic");
}

#[test]
fn category_ignore_suppresses_only_that_category() {
    let text = r#"<!-- dwg:ignore buzzword -->
Additionally, we will leverage the system.
<!-- dwg:end-ignore -->"#;
    let report = analyze(text);
    assert_not(&report, Category::Buzzword);
    assert_has(&report, Category::Transition);
}

#[test]
fn category_ignore_allows_multiple_categories() {
    let text = r#"<!-- dwg:ignore buzzword, transition -->
Additionally, we will leverage the system.
<!-- dwg:end-ignore -->
Additionally, we will leverage the system."#;
    let report = analyze(text);
    let buzzwords = report
        .diagnostics
        .iter()
        .filter(|d| d.category == Category::Buzzword)
        .count();
    let transitions = report
        .diagnostics
        .iter()
        .filter(|d| d.category == Category::Transition)
        .count();
    assert_eq!(buzzwords, 1, "expected only 1 buzzword diagnostic");
    assert_eq!(transitions, 1, "expected only 1 transition diagnostic");
}
