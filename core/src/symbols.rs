//! Cross-file symbol graph for static tracing and usage analysis.
//!
//! This module builds a project-wide symbol table that tracks:
//! - Exports from each file
//! - Imports into each file
//! - Type usages (where each type/interface is referenced)
//! - Call sites (where each function is called)
//!
//! This enables "logic gating" to reduce false positives by checking
//! if abstractions are actually used before flagging them.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How a symbol is being used
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsageKind {
    /// Used as a type annotation: `const x: MyInterface = ...`
    TypeAnnotation,
    /// Implemented by a class: `class X implements MyInterface`
    Implements,
    /// Extended by a class/interface: `class X extends Base`
    Extends,
    /// Called as a function: `myFunction()`
    Call,
    /// Imported into a file
    Import,
    /// Exported from a file
    Export,
    /// Instantiated with new: `new MyClass()`
    Instantiate,
    /// Passed as a type parameter: `Map<string, MyInterface>`
    TypeParameter,
    /// Unknown usage type
    Unknown,
}

/// A site where a symbol is used
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSite {
    /// File path where the usage occurs
    pub path: PathBuf,
    /// Line number (1-based)
    pub line: u32,
    /// Column number (1-based)
    pub column: Option<u32>,
    /// How the symbol is being used
    pub usage_kind: UsageKind,
    /// The symbol being used
    pub symbol_name: String,
    /// Context (e.g., the function name where usage occurs)
    pub context: Option<String>,
}

/// An exported symbol from a file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedSymbol {
    /// Name of the exported symbol
    pub name: String,
    /// Kind: function, class, interface, type, const, etc.
    pub kind: String,
    /// Line number where defined
    pub line: u32,
    /// Whether it's a default export
    pub is_default: bool,
    /// Whether it's re-exported from another module
    pub is_reexport: bool,
}

/// An import reference in a file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRef {
    /// The imported name (local binding)
    pub local_name: String,
    /// The original name from the source module
    pub imported_name: Option<String>,
    /// The source module path
    pub source_module: String,
    /// Line number
    pub line: u32,
    /// Whether it's a namespace import: `import * as ns from '...'`
    pub is_namespace: bool,
    /// Whether it's a default import
    pub is_default: bool,
}

/// A call site where a function is invoked
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallSite {
    /// The function being called
    pub callee: String,
    /// File path where the call occurs
    pub path: PathBuf,
    /// Line number
    pub line: u32,
    /// Arguments count
    pub arg_count: usize,
    /// Whether it's a method call (receiver.method())
    pub is_method_call: bool,
    /// The receiver type if it's a method call
    pub receiver_type: Option<String>,
}

/// Project-wide symbol graph for cross-file analysis
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SymbolGraph {
    /// Exports from each file: path -> list of exports
    pub exports: HashMap<PathBuf, Vec<ExportedSymbol>>,
    
    /// Imports into each file: path -> list of imports
    pub imports: HashMap<PathBuf, Vec<ImportRef>>,
    
    /// Type usages by symbol name: symbol_name -> list of usage sites
    pub type_usages: HashMap<String, Vec<UsageSite>>,
    
    /// Call sites by function name: function_name -> list of call sites
    pub call_sites: HashMap<String, Vec<CallSite>>,
    
    /// Entry points detected in the project
    pub entry_points: Vec<EntryPoint>,
    
    /// Reachable symbols from entry points (computed lazily)
    #[serde(skip)]
    pub reachable_cache: Option<HashSet<String>>,
}

/// An entry point in the codebase
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryPoint {
    /// Path to the entry point file
    pub path: PathBuf,
    /// Name of the entry point (e.g., main, default export)
    pub name: String,
    /// Kind of entry point
    pub kind: EntryPointKind,
    /// Line number
    pub line: u32,
}

/// Types of entry points
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntryPointKind {
    /// Main function (Rust, Python)
    Main,
    /// Default export (JavaScript/TypeScript)
    DefaultExport,
    /// Named export from index file
    IndexExport,
    /// Route handler (Express, Fastify, etc.)
    RouteHandler,
    /// CLI command
    CliCommand,
    /// Test function
    Test,
    /// Job/worker handler
    JobHandler,
    /// Event handler
    EventHandler,
    /// Other
    Other,
}

impl SymbolGraph {
    /// Add an export to the graph
    pub fn add_export(&mut self, path: PathBuf, export: ExportedSymbol) {
        self.exports.entry(path).or_default().push(export);
    }

    /// Add an import to the graph
    pub fn add_import(&mut self, path: PathBuf, import: ImportRef) {
        self.imports.entry(path).or_default().push(import);
    }

    /// Add a type usage to the graph
    pub fn add_type_usage(&mut self, symbol_name: String, usage: UsageSite) {
        self.type_usages.entry(symbol_name).or_default().push(usage);
    }

    /// Add a call site to the graph
    pub fn add_call_site(&mut self, callee: String, call: CallSite) {
        self.call_sites.entry(callee).or_default().push(call);
    }

    /// Add an entry point
    pub fn add_entry_point(&mut self, entry: EntryPoint) {
        self.entry_points.push(entry);
    }

    /// Get all usages of a type/interface
    pub fn get_type_usages(&self, name: &str) -> Vec<&UsageSite> {
        self.type_usages
            .get(name)
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// Get all call sites for a function
    pub fn get_call_sites(&self, name: &str) -> Vec<&CallSite> {
        self.call_sites
            .get(name)
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// Check if a type is used as a type annotation anywhere
    pub fn is_used_as_type_annotation(&self, name: &str) -> bool {
        self.type_usages
            .get(name)
            .map(|usages| usages.iter().any(|u| u.usage_kind == UsageKind::TypeAnnotation))
            .unwrap_or(false)
    }

    /// Check if a type is implemented anywhere
    pub fn is_implemented(&self, name: &str) -> bool {
        self.type_usages
            .get(name)
            .map(|usages| usages.iter().any(|u| u.usage_kind == UsageKind::Implements))
            .unwrap_or(false)
    }

    /// Get implementation count for a type
    pub fn implementation_count(&self, name: &str) -> usize {
        self.type_usages
            .get(name)
            .map(|usages| usages.iter().filter(|u| u.usage_kind == UsageKind::Implements).count())
            .unwrap_or(0)
    }

    /// Get type annotation usage count for a type
    pub fn type_annotation_count(&self, name: &str) -> usize {
        self.type_usages
            .get(name)
            .map(|usages| usages.iter().filter(|u| u.usage_kind == UsageKind::TypeAnnotation).count())
            .unwrap_or(0)
    }

    /// Compute reachability from entry points using BFS
    pub fn compute_reachability(&mut self) -> HashSet<String> {
        if let Some(ref cache) = self.reachable_cache {
            return cache.clone();
        }

        let mut reachable = HashSet::new();
        let mut queue: Vec<String> = Vec::new();

        // Start with entry point symbols
        for entry in &self.entry_points {
            queue.push(entry.name.clone());
        }

        // BFS through call graph
        while let Some(current) = queue.pop() {
            if reachable.contains(&current) {
                continue;
            }
            reachable.insert(current.clone());

            // Find all calls from this symbol (simplified - assumes call sites
            // are indexed by caller, not callee)
            // In practice, this would need more sophisticated call graph analysis
        }

        self.reachable_cache = Some(reachable.clone());
        reachable
    }

    /// Check if a symbol is reachable from entry points
    pub fn is_reachable(&self, name: &str) -> bool {
        if let Some(ref cache) = self.reachable_cache {
            cache.contains(name)
        } else {
            // If reachability hasn't been computed, assume reachable
            true
        }
    }

    /// Get statistics about the symbol graph
    pub fn stats(&self) -> SymbolGraphStats {
        SymbolGraphStats {
            files_with_exports: self.exports.len(),
            total_exports: self.exports.values().map(|v| v.len()).sum(),
            files_with_imports: self.imports.len(),
            total_imports: self.imports.values().map(|v| v.len()).sum(),
            unique_types_used: self.type_usages.len(),
            total_type_usages: self.type_usages.values().map(|v| v.len()).sum(),
            unique_functions_called: self.call_sites.len(),
            total_call_sites: self.call_sites.values().map(|v| v.len()).sum(),
            entry_points: self.entry_points.len(),
        }
    }
}

/// Statistics about the symbol graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolGraphStats {
    pub files_with_exports: usize,
    pub total_exports: usize,
    pub files_with_imports: usize,
    pub total_imports: usize,
    pub unique_types_used: usize,
    pub total_type_usages: usize,
    pub unique_functions_called: usize,
    pub total_call_sites: usize,
    pub entry_points: usize,
}

/// Confidence score for a finding based on usage analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceScore {
    /// Overall score from 0.0 to 1.0
    pub score: f32,
    /// Individual factors that contributed to the score
    pub factors: Vec<ConfidenceFactor>,
}

/// A factor that contributes to confidence scoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceFactor {
    /// Name of the factor
    pub name: String,
    /// Weight applied (positive = increases confidence, negative = decreases)
    pub weight: f32,
    /// Description of why this factor applies
    pub reason: String,
}

impl ConfidenceScore {
    /// Create a new confidence score
    pub fn new(factors: Vec<ConfidenceFactor>) -> Self {
        let score = factors.iter().map(|f| f.weight).sum::<f32>();
        let clamped = score.clamp(0.0, 1.0);
        Self {
            score: clamped,
            factors,
        }
    }

    /// Check if this is high confidence (>0.7)
    pub fn is_high(&self) -> bool {
        self.score > 0.7
    }

    /// Check if this is medium confidence (>0.5)
    pub fn is_medium(&self) -> bool {
        self.score > 0.5
    }
}

/// Build a symbol graph from TypeScript/JavaScript files
pub fn build_ts_symbol_graph(files: &[(PathBuf, String)]) -> SymbolGraph {
    use tree_sitter::Parser;

    let mut graph = SymbolGraph::default();
    let mut parser = Parser::new();
    let lang = tree_sitter_typescript::LANGUAGE_TYPESCRIPT;
    
    if parser.set_language(&lang.into()).is_err() {
        return graph;
    }

    for (path, content) in files {
        let Some(tree) = parser.parse(content, None) else {
            continue;
        };

        let root = tree.root_node();
        
        // Extract exports, imports, and usages
        extract_ts_symbols(&mut graph, path, &root, content);
    }

    graph
}

/// Extract symbols from a TypeScript AST
fn extract_ts_symbols(
    graph: &mut SymbolGraph,
    path: &Path,
    root: &tree_sitter::Node,
    content: &str,
) {
    use tree_sitter::Node;

    fn node_text<'a>(node: Node<'a>, src: &'a str) -> String {
        node.utf8_text(src.as_bytes()).unwrap_or("").to_string()
    }

    let mut cursor = root.walk();
    let mut stack = vec![*root];

    while let Some(node) = stack.pop() {
        match node.kind() {
            // Export declarations
            "export_statement" => {
                if let Some(decl) = node.child_by_field_name("declaration") {
                    let kind = decl.kind().to_string();
                    let name = decl
                        .child_by_field_name("name")
                        .map(|n| node_text(n, content))
                        .unwrap_or_default();
                    
                    if !name.is_empty() {
                        graph.add_export(
                            path.to_path_buf(),
                            ExportedSymbol {
                                name: name.clone(),
                                kind,
                                line: node.start_position().row as u32 + 1,
                                is_default: false,
                                is_reexport: false,
                            },
                        );

                        // If it's a function, it might be an entry point
                        if decl.kind() == "function_declaration" && name == "main" {
                            graph.add_entry_point(EntryPoint {
                                path: path.to_path_buf(),
                                name: name.clone(),
                                kind: EntryPointKind::Main,
                                line: node.start_position().row as u32 + 1,
                            });
                        }
                    }
                }
            }

            // Import declarations
            "import_statement" => {
                if let Some(source) = node.child_by_field_name("source") {
                    let source_module = node_text(source, content)
                        .trim_matches(|c| c == '"' || c == '\'')
                        .to_string();
                    
                    // Find imported names
                    if let Some(clause) = node.child_by_field_name("import_clause") {
                        for i in 0..clause.child_count() {
                            if let Some(child) = clause.child(i) {
                                if child.kind() == "identifier" {
                                    graph.add_import(
                                        path.to_path_buf(),
                                        ImportRef {
                                            local_name: node_text(child, content),
                                            imported_name: None,
                                            source_module: source_module.clone(),
                                            line: node.start_position().row as u32 + 1,
                                            is_namespace: false,
                                            is_default: true,
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
            }

            // Type annotations - track type usages
            "type_annotation" => {
                // The type_annotation contains the type being used
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "type_identifier" {
                            let type_name = node_text(child, content);
                            graph.add_type_usage(
                                type_name.clone(),
                                UsageSite {
                                    path: path.to_path_buf(),
                                    line: child.start_position().row as u32 + 1,
                                    column: Some(child.start_position().column as u32 + 1),
                                    usage_kind: UsageKind::TypeAnnotation,
                                    symbol_name: type_name,
                                    context: None,
                                },
                            );
                        }
                    }
                }
            }

            // Implements clause
            "implements_clause" => {
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "type_identifier" {
                            let type_name = node_text(child, content);
                            graph.add_type_usage(
                                type_name.clone(),
                                UsageSite {
                                    path: path.to_path_buf(),
                                    line: child.start_position().row as u32 + 1,
                                    column: Some(child.start_position().column as u32 + 1),
                                    usage_kind: UsageKind::Implements,
                                    symbol_name: type_name,
                                    context: None,
                                },
                            );
                        }
                    }
                }
            }

            // Call expressions - track function calls
            "call_expression" => {
                if let Some(func) = node.child_by_field_name("function") {
                    let callee = node_text(func, content);
                    let args = node.child_by_field_name("arguments");
                    let arg_count = args.map(|a| a.child_count().saturating_sub(2) / 2).unwrap_or(0);
                    
                    graph.add_call_site(
                        callee.clone(),
                        CallSite {
                            callee,
                            path: path.to_path_buf(),
                            line: node.start_position().row as u32 + 1,
                            arg_count,
                            is_method_call: func.kind() == "member_expression",
                            receiver_type: None,
                        },
                    );
                }
            }

            _ => {}
        }

        // Continue traversing
        if node.child_count() > 0 {
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_graph_basic() {
        let mut graph = SymbolGraph::default();
        
        graph.add_type_usage(
            "MyInterface".to_string(),
            UsageSite {
                path: PathBuf::from("test.ts"),
                line: 10,
                column: Some(5),
                usage_kind: UsageKind::TypeAnnotation,
                symbol_name: "MyInterface".to_string(),
                context: None,
            },
        );

        assert!(graph.is_used_as_type_annotation("MyInterface"));
        assert!(!graph.is_used_as_type_annotation("OtherInterface"));
        assert_eq!(graph.type_annotation_count("MyInterface"), 1);
    }

    #[test]
    fn test_confidence_score() {
        let factors = vec![
            ConfidenceFactor {
                name: "reachable".to_string(),
                weight: 0.3,
                reason: "Reachable from entry point".to_string(),
            },
            ConfidenceFactor {
                name: "not_exported".to_string(),
                weight: 0.2,
                reason: "Internal symbol".to_string(),
            },
        ];
        
        let score = ConfidenceScore::new(factors);
        assert!((score.score - 0.5).abs() < 0.001);
        assert!(!score.is_high());
        assert!(!score.is_medium());
    }
}
