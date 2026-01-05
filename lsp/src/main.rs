//! ToneGuard Language Server Protocol implementation.
//!
//! This provides an in-process LSP server that keeps the analyzer hot in memory
//! and provides real-time diagnostics, code actions, and configuration support.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use dashmap::DashMap;
use dwg_core::{Analyzer, Category, Config, Diagnostic as CoreDiagnostic, Severity};
use serde_json::Value;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

/// Document state cached by the server.
struct DocumentState {
    content: String,
    version: i32,
}

#[derive(Clone, Default)]
struct CategoryFilter {
    only: HashSet<Category>,
    enable: HashSet<Category>,
    disable: HashSet<Category>,
}

impl CategoryFilter {
    fn allows(&self, category: Category) -> bool {
        if !self.only.is_empty() {
            return self.only.contains(&category);
        }
        if !self.enable.is_empty() {
            return !self.disable.contains(&category) || self.enable.contains(&category);
        }
        !self.disable.contains(&category)
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

/// ToneGuard Language Server backend.
struct Backend {
    client: Client,
    analyzer: RwLock<Arc<Analyzer>>,
    documents: DashMap<Url, DocumentState>,
    workspace_root: RwLock<Option<PathBuf>>,
    config_path: RwLock<Option<PathBuf>>,
    forced_profile: RwLock<Option<String>>,
    category_filter: RwLock<CategoryFilter>,
}

impl Backend {
    fn new(client: Client) -> Self {
        let config = Config::default();
        let analyzer = Analyzer::new(config).expect("failed to create analyzer");
        Self {
            client,
            analyzer: RwLock::new(Arc::new(analyzer)),
            documents: DashMap::new(),
            workspace_root: RwLock::new(None),
            config_path: RwLock::new(None),
            forced_profile: RwLock::new(None),
            category_filter: RwLock::new(CategoryFilter::default()),
        }
    }

    fn load_config(path: &Path) -> anyhow::Result<Config> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let value: serde_yaml::Value = serde_yaml::from_str(&text)
            .with_context(|| format!("Failed to parse YAML {}", path.display()))?;
        let cfg: Config = serde_yaml::from_value(value)
            .with_context(|| format!("Invalid config structure in {}", path.display()))?;
        Ok(cfg)
    }

    async fn reload_analyzer(&self) -> anyhow::Result<()> {
        let workspace_root = self.workspace_root.read().await.clone();
        let Some(workspace_root) = workspace_root else {
            return Ok(());
        };

        let configured = self.config_path.read().await.clone();
        let resolved = configured.unwrap_or_else(|| workspace_root.join("layth-style.yml"));

        let cfg = if resolved.exists() {
            Self::load_config(&resolved)?
        } else {
            Config::default()
        };

        let analyzer = Analyzer::new(cfg).context("failed to create analyzer")?;
        *self.analyzer.write().await = Arc::new(analyzer);
        *self.config_path.write().await = Some(resolved.clone());

        self.client
            .log_message(
                MessageType::INFO,
                format!("ToneGuard config loaded: {}", resolved.display()),
            )
            .await;

        Ok(())
    }

    /// Analyze a document and return LSP diagnostics.
    async fn analyze_document(&self, uri: &Url) -> Vec<tower_lsp::lsp_types::Diagnostic> {
        let Some(doc) = self.documents.get(uri) else {
            return vec![];
        };
        let content = &doc.content;

        let analyzer = self.analyzer.read().await.clone();

        let profile_name = self.profile_for_uri(&analyzer, uri).await;
        let report = analyzer
            .analyze_profile_name(content, &profile_name)
            .unwrap_or_else(|_| analyzer.analyze(content));

        let filter = self.category_filter.read().await.clone();
        report
            .diagnostics
            .into_iter()
            .filter(|d| filter.allows(d.category))
            .map(|d| self.to_lsp_diagnostic(&d, content))
            .collect()
    }

    async fn profile_for_uri(&self, analyzer: &Analyzer, uri: &Url) -> String {
        if let Some(forced) = self.forced_profile.read().await.clone() {
            if !forced.trim().is_empty() {
                return forced;
            }
        }

        let Some(path) = uri.to_file_path().ok() else {
            return analyzer.default_profile().to_string();
        };
        let root = self.workspace_root.read().await.clone();
        let relative = if let Some(root) = root {
            path.strip_prefix(&root)
                .unwrap_or(path.as_path())
                .to_path_buf()
        } else {
            path.clone()
        };
        let relative_str = relative
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        analyzer.profile_for_path(&relative_str).to_string()
    }

    fn byte_to_position(text: &str, byte_offset: usize) -> Position {
        let byte_offset = byte_offset.min(text.len());
        let mut line: u32 = 0;
        let mut last_newline = 0usize;
        for (idx, ch) in text.char_indices() {
            if idx >= byte_offset {
                break;
            }
            if ch == '\n' {
                line += 1;
                last_newline = idx + 1;
            }
        }
        let character = text[last_newline..byte_offset].encode_utf16().count() as u32;
        Position { line, character }
    }

    /// Convert a core diagnostic to an LSP diagnostic.
    fn to_lsp_diagnostic(
        &self,
        diag: &CoreDiagnostic,
        text: &str,
    ) -> tower_lsp::lsp_types::Diagnostic {
        let range = Range {
            start: Self::byte_to_position(text, diag.span.0),
            end: Self::byte_to_position(text, diag.span.1),
        };

        let severity = match diag.severity {
            Severity::Error => DiagnosticSeverity::ERROR,
            Severity::Warning => DiagnosticSeverity::WARNING,
            Severity::Hint => DiagnosticSeverity::HINT,
            Severity::Information => DiagnosticSeverity::INFORMATION,
        };

        let mut message = format!("[{}] {}", diag.category, diag.message);
        if let Some(ref suggestion) = diag.suggestion {
            message.push_str(" → ");
            message.push_str(suggestion);
        }

        tower_lsp::lsp_types::Diagnostic {
            range,
            severity: Some(severity),
            code: Some(NumberOrString::String(format!("{:?}", diag.category))),
            code_description: None,
            source: Some("toneguard".to_string()),
            message,
            related_information: None,
            tags: None,
            data: None,
        }
    }

    /// Publish diagnostics to the client.
    async fn publish_diagnostics(&self, uri: Url) {
        let diagnostics = self.analyze_document(&uri).await;
        let version = self.documents.get(&uri).map(|d| d.version);
        self.client
            .publish_diagnostics(uri, diagnostics, version)
            .await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        if let Some(root_uri) = params.root_uri.or_else(|| {
            params
                .workspace_folders
                .as_ref()
                .and_then(|folders| folders.first().map(|f| f.uri.clone()))
        }) {
            if let Ok(path) = root_uri.to_file_path() {
                *self.workspace_root.write().await = Some(path);
            }
        }

        if let Some(Value::Object(map)) = params.initialization_options {
            if let Some(Value::String(config_path)) = map.get("configPath") {
                if !config_path.trim().is_empty() {
                    let configured = PathBuf::from(config_path);
                    if configured.is_absolute() {
                        *self.config_path.write().await = Some(configured);
                    } else if let Some(root) = self.workspace_root.read().await.clone() {
                        *self.config_path.write().await = Some(root.join(configured));
                    }
                }
            }
            if let Some(Value::String(profile)) = map.get("profile") {
                if profile.trim().is_empty() {
                    *self.forced_profile.write().await = None;
                } else {
                    *self.forced_profile.write().await = Some(profile.clone());
                }
            }

            let mut filter = CategoryFilter::default();
            if let Some(Value::Array(items)) = map.get("onlyCategories") {
                for item in items {
                    if let Some(name) = item.as_str() {
                        if let Some(cat) = parse_category(name) {
                            filter.only.insert(cat);
                        }
                    }
                }
            }
            if let Some(Value::Array(items)) = map.get("enableCategories") {
                for item in items {
                    if let Some(name) = item.as_str() {
                        if let Some(cat) = parse_category(name) {
                            filter.enable.insert(cat);
                        }
                    }
                }
            }
            if let Some(Value::Array(items)) = map.get("disableCategories") {
                for item in items {
                    if let Some(name) = item.as_str() {
                        if let Some(cat) = parse_category(name) {
                            filter.disable.insert(cat);
                        }
                    }
                }
            }
            *self.category_filter.write().await = filter;
        }

        if let Err(err) = self.reload_analyzer().await {
            self.client
                .log_message(
                    MessageType::ERROR,
                    format!("Failed to load config: {err:#}"),
                )
                .await;
        }

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                // We use push diagnostics via publish_diagnostics(), not pull diagnostics
                // Don't advertise diagnostic_provider to avoid -32601 errors
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![
                            CodeActionKind::QUICKFIX,
                            CodeActionKind::REFACTOR,
                        ]),
                        work_done_progress_options: WorkDoneProgressOptions {
                            work_done_progress: None,
                        },
                        resolve_provider: Some(false),
                    },
                )),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "ToneGuard Language Server".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "ToneGuard LSP initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let content = params.text_document.text;
        let version = params.text_document.version;

        self.documents
            .insert(uri.clone(), DocumentState { content, version });

        self.publish_diagnostics(uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = params.text_document.version;

        // With FULL sync, we get the complete new content
        if let Some(change) = params.content_changes.into_iter().last() {
            self.documents.insert(
                uri.clone(),
                DocumentState {
                    content: change.text,
                    version,
                },
            );
        }

        self.publish_diagnostics(uri).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        let saved_path = uri.to_file_path().ok();
        let config_path = self.config_path.read().await.clone();
        let is_config = saved_path
            .as_ref()
            .zip(config_path.as_ref())
            .is_some_and(|(a, b)| a == b);

        if is_config {
            if let Err(err) = self.reload_analyzer().await {
                self.client
                    .log_message(
                        MessageType::ERROR,
                        format!("Failed to reload config: {err:#}"),
                    )
                    .await;
            }
            let uris: Vec<Url> = self.documents.iter().map(|e| e.key().clone()).collect();
            for uri in uris {
                self.publish_diagnostics(uri).await;
            }
        } else {
            self.publish_diagnostics(uri).await;
        }
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        if let Value::Object(map) = params.settings {
            if let Some(Value::String(config_path)) = map.get("configPath") {
                if config_path.trim().is_empty() {
                    *self.config_path.write().await = None;
                } else {
                    let configured = PathBuf::from(config_path);
                    if configured.is_absolute() {
                        *self.config_path.write().await = Some(configured);
                    } else if let Some(root) = self.workspace_root.read().await.clone() {
                        *self.config_path.write().await = Some(root.join(configured));
                    }
                }
            }

            if let Some(Value::String(profile)) = map.get("profile") {
                if profile.trim().is_empty() {
                    *self.forced_profile.write().await = None;
                } else {
                    *self.forced_profile.write().await = Some(profile.clone());
                }
            }

            let mut filter = CategoryFilter::default();
            if let Some(Value::Array(items)) = map.get("onlyCategories") {
                for item in items {
                    if let Some(name) = item.as_str() {
                        if let Some(cat) = parse_category(name) {
                            filter.only.insert(cat);
                        }
                    }
                }
            }
            if let Some(Value::Array(items)) = map.get("enableCategories") {
                for item in items {
                    if let Some(name) = item.as_str() {
                        if let Some(cat) = parse_category(name) {
                            filter.enable.insert(cat);
                        }
                    }
                }
            }
            if let Some(Value::Array(items)) = map.get("disableCategories") {
                for item in items {
                    if let Some(name) = item.as_str() {
                        if let Some(cat) = parse_category(name) {
                            filter.disable.insert(cat);
                        }
                    }
                }
            }
            *self.category_filter.write().await = filter;
        }

        if let Err(err) = self.reload_analyzer().await {
            self.client
                .log_message(
                    MessageType::ERROR,
                    format!("Failed to reload config: {err:#}"),
                )
                .await;
        }
        let uris: Vec<Url> = self.documents.iter().map(|e| e.key().clone()).collect();
        for uri in uris {
            self.publish_diagnostics(uri).await;
        }
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        let config_path = self.config_path.read().await.clone();
        let mut should_reload = false;

        for change in &params.changes {
            if let Some(config_path) = &config_path {
                if let Ok(path) = change.uri.to_file_path() {
                    if &path == config_path {
                        should_reload = true;
                        break;
                    }
                }
            } else if change.uri.path().ends_with("layth-style.yml") {
                should_reload = true;
                break;
            }
        }

        if !should_reload {
            return;
        }

        if let Err(err) = self.reload_analyzer().await {
            self.client
                .log_message(
                    MessageType::ERROR,
                    format!("Failed to reload config: {err:#}"),
                )
                .await;
        }
        let uris: Vec<Url> = self.documents.iter().map(|e| e.key().clone()).collect();
        for uri in uris {
            self.publish_diagnostics(uri).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents.remove(&params.text_document.uri);
        // Clear diagnostics
        self.client
            .publish_diagnostics(params.text_document.uri, vec![], None)
            .await;
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = &params.text_document.uri;
        if !self.documents.contains_key(uri) {
            return Ok(None);
        }

        let mut actions = Vec::new();

        // Add "Ignore this line" action for each diagnostic
        for diag in &params.context.diagnostics {
            if diag.source.as_deref() == Some("toneguard") {
                // Create an "Ignore line" action
                let line = diag.range.start.line;
                let insert_char = self
                    .documents
                    .get(uri)
                    .and_then(|doc| {
                        doc.content
                            .lines()
                            .nth(line as usize)
                            .map(|l| l.encode_utf16().count() as u32)
                    })
                    .unwrap_or(0);

                let edit = TextEdit {
                    range: Range {
                        start: Position {
                            line,
                            character: insert_char,
                        },
                        end: Position {
                            line,
                            character: insert_char,
                        },
                    },
                    new_text: " <!-- dwg:ignore-line -->".to_string(),
                };

                let mut changes = HashMap::new();
                changes.insert(uri.clone(), vec![edit]);

                let action = CodeAction {
                    title: "Ignore this line (ToneGuard)".to_string(),
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: Some(vec![diag.clone()]),
                    edit: Some(WorkspaceEdit {
                        changes: Some(changes),
                        ..Default::default()
                    }),
                    command: None,
                    is_preferred: Some(false),
                    disabled: None,
                    data: None,
                };

                actions.push(CodeActionOrCommand::CodeAction(action));

                // Add "Disable all checks" action
                let mut changes2 = HashMap::new();
                let edit2 = TextEdit {
                    range: Range {
                        start: Position { line, character: 0 },
                        end: Position { line, character: 0 },
                    },
                    new_text: "<!-- dwg:off -->\n".to_string(),
                };
                changes2.insert(uri.clone(), vec![edit2]);

                let action2 = CodeAction {
                    title: "Disable ToneGuard from here".to_string(),
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: Some(vec![diag.clone()]),
                    edit: Some(WorkspaceEdit {
                        changes: Some(changes2),
                        ..Default::default()
                    }),
                    command: None,
                    is_preferred: Some(false),
                    disabled: None,
                    data: None,
                };

                actions.push(CodeActionOrCommand::CodeAction(action2));
            }
        }

        if actions.is_empty() {
            Ok(None)
        } else {
            Ok(Some(actions))
        }
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
