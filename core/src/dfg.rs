//! Data Flow Graph (DFG) construction and analysis.
//!
//! This module tracks how values flow through a program:
//! - Where variables are defined (def sites)
//! - Where variables are used (use sites)
//! - How values transform from definition to use
//!
//! This enables detection of:
//! - Error escalation patterns (warnings becoming errors)
//! - Unchecked inputs (validation gaps)
//! - Unused definitions

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use syn::spanned::Spanned;

use serde::{Deserialize, Serialize};

/// Data Flow Graph for a single function/method
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataFlowGraph {
    /// Function/method name
    pub name: String,
    /// File path
    pub path: PathBuf,
    /// All definitions (where variables are assigned values)
    pub definitions: HashMap<String, Vec<DefSite>>,
    /// All usages (where variables are read)
    pub usages: HashMap<String, Vec<UseSite>>,
    /// Data flow chains (def -> use relationships)
    pub flows: Vec<DataFlow>,
    /// Parameters (function inputs)
    pub parameters: Vec<Parameter>,
}

/// Where a variable is defined/assigned
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefSite {
    /// Variable name
    pub variable: String,
    /// Line number
    pub line: u32,
    /// Column number (optional)
    pub column: Option<u32>,
    /// Kind of definition
    pub kind: DefKind,
    /// The expression that provides the value (if simple enough to extract)
    pub value_expr: Option<String>,
    /// Whether this definition is conditional
    pub is_conditional: bool,
}

/// Kind of definition
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DefKind {
    /// Parameter in function signature
    Parameter,
    /// Local variable declaration (let, const, var)
    Declaration,
    /// Assignment to existing variable
    Assignment,
    /// Compound assignment (+=, -=, etc.)
    CompoundAssignment,
    /// For loop iterator variable
    ForIterator,
    /// Destructuring assignment
    Destructure,
    /// Function/closure definition
    Function,
    /// Import binding
    Import,
    /// Default value (fallback when None/undefined)
    Default,
}

/// Where a variable is used/read
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UseSite {
    /// Variable name
    pub variable: String,
    /// Line number
    pub line: u32,
    /// Column number (optional)
    pub column: Option<u32>,
    /// Kind of usage
    pub kind: UseKind,
    /// Context: what operation is the value used in
    pub context: Option<String>,
}

/// Kind of usage
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UseKind {
    /// Used in a condition (if, while, etc.)
    Condition,
    /// Used as a function argument
    Argument,
    /// Used on right-hand side of assignment
    Read,
    /// Used for method call receiver
    Receiver,
    /// Used in return statement
    Return,
    /// Used in assertion
    Assertion,
    /// Used in comparison
    Comparison,
    /// Used in arithmetic
    Arithmetic,
    /// Used as index
    Index,
    /// Used in field access
    FieldAccess,
    /// Other usage
    Other,
}

/// A data flow from definition to usage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataFlow {
    /// Variable name
    pub variable: String,
    /// Where the value originates
    pub from: DefSite,
    /// Where the value is consumed
    pub to: UseSite,
    /// Transformations applied along the way
    pub transformations: Vec<Transformation>,
    /// Whether this flow crosses a conditional boundary
    pub conditional: bool,
}

/// A transformation applied to a value
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transformation {
    /// Kind of transformation
    pub kind: TransformKind,
    /// Line where transformation occurs
    pub line: u32,
    /// Description of the transformation
    pub description: String,
}

/// Kind of value transformation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransformKind {
    /// Increment (+=, ++)
    Increment,
    /// Decrement (-=, --)
    Decrement,
    /// Unwrap (Option/Result unwrapping)
    Unwrap,
    /// Expect (with error message)
    Expect,
    /// Type cast/conversion
    Cast,
    /// Method call that transforms the value
    MethodCall,
    /// Arithmetic operation
    Arithmetic,
    /// String operation
    String,
    /// Comparison that yields boolean
    Comparison,
    /// Default value fallback
    DefaultFallback,
    /// Validation/check
    Validation,
    /// Clone/copy
    Clone,
    /// Reference/borrow
    Reference,
    /// Dereference
    Dereference,
}

/// A function parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parameter {
    /// Parameter name
    pub name: String,
    /// Parameter type (if available)
    pub param_type: Option<String>,
    /// Line number
    pub line: u32,
    /// Whether it has a default value
    pub has_default: bool,
    /// Whether it's optional (e.g., Option<T>, ?)
    pub is_optional: bool,
}

impl DataFlowGraph {
    /// Create a new empty DFG
    pub fn new(name: String, path: PathBuf) -> Self {
        Self {
            name,
            path,
            definitions: HashMap::new(),
            usages: HashMap::new(),
            flows: Vec::new(),
            parameters: Vec::new(),
        }
    }

    /// Add a definition site
    pub fn add_definition(&mut self, def: DefSite) {
        self.definitions
            .entry(def.variable.clone())
            .or_default()
            .push(def);
    }

    /// Add a usage site
    pub fn add_usage(&mut self, usage: UseSite) {
        self.usages
            .entry(usage.variable.clone())
            .or_default()
            .push(usage);
    }

    /// Add a parameter
    pub fn add_parameter(&mut self, param: Parameter) {
        // Also add as a definition
        self.add_definition(DefSite {
            variable: param.name.clone(),
            line: param.line,
            column: None,
            kind: DefKind::Parameter,
            value_expr: None,
            is_conditional: false,
        });
        self.parameters.push(param);
    }

    /// Build data flow chains (connect definitions to usages)
    pub fn build_flows(&mut self) {
        self.flows.clear();

        for (var, usages) in &self.usages {
            if let Some(defs) = self.definitions.get(var) {
                for usage in usages {
                    // Find the most recent definition before this usage
                    let def = defs
                        .iter()
                        .filter(|d| d.line <= usage.line)
                        .max_by_key(|d| d.line);

                    if let Some(def) = def {
                        self.flows.push(DataFlow {
                            variable: var.clone(),
                            from: def.clone(),
                            to: usage.clone(),
                            transformations: Vec::new(),
                            conditional: def.is_conditional,
                        });
                    }
                }
            }
        }
    }

    /// Find all usages of a variable after a given line
    pub fn usages_after(&self, variable: &str, line: u32) -> Vec<&UseSite> {
        self.usages
            .get(variable)
            .map(|usages| usages.iter().filter(|u| u.line > line).collect())
            .unwrap_or_default()
    }

    /// Find all definitions of a variable
    pub fn definitions_of(&self, variable: &str) -> Vec<&DefSite> {
        self.definitions
            .get(variable)
            .map(|defs| defs.iter().collect())
            .unwrap_or_default()
    }

    /// Find parameters that are used without validation
    pub fn unvalidated_params(&self) -> Vec<&Parameter> {
        self.parameters
            .iter()
            .filter(|p| !self.has_validation(&p.name))
            .collect()
    }

    /// Check if a variable has validation before dangerous use
    fn has_validation(&self, variable: &str) -> bool {
        // Check if the variable is used in an assertion or condition
        // before being used in a dangerous operation
        if let Some(usages) = self.usages.get(variable) {
            let has_check = usages.iter().any(|u| {
                matches!(
                    u.kind,
                    UseKind::Condition | UseKind::Assertion | UseKind::Comparison
                )
            });

            let has_dangerous_use = usages.iter().any(|u| matches!(u.kind, UseKind::Index));

            // Has validation if checked before dangerous use
            if has_check && has_dangerous_use {
                let check_line = usages
                    .iter()
                    .filter(|u| {
                        matches!(
                            u.kind,
                            UseKind::Condition | UseKind::Assertion | UseKind::Comparison
                        )
                    })
                    .map(|u| u.line)
                    .min();

                let danger_line = usages
                    .iter()
                    .filter(|u| matches!(u.kind, UseKind::Index))
                    .map(|u| u.line)
                    .min();

                if let (Some(check), Some(danger)) = (check_line, danger_line) {
                    return check < danger;
                }
            }

            has_check
        } else {
            true // No usages, assume validated
        }
    }

    /// Find error escalation patterns
    /// (where a variable used with "warning" is later used with "error")
    pub fn find_error_escalation(&self) -> Vec<EscalationPattern> {
        let mut patterns = Vec::new();

        // Look for variables that are used in both warning and error contexts
        for (var, usages) in &self.usages {
            let warning_uses: Vec<_> = usages
                .iter()
                .filter(|u| {
                    u.context
                        .as_ref()
                        .map(|c| c.to_lowercase().contains("warning"))
                        .unwrap_or(false)
                })
                .collect();

            let error_uses: Vec<_> = usages
                .iter()
                .filter(|u| {
                    u.context
                        .as_ref()
                        .map(|c| c.to_lowercase().contains("error"))
                        .unwrap_or(false)
                })
                .collect();

            if !warning_uses.is_empty() && !error_uses.is_empty() {
                for warning in &warning_uses {
                    for error in &error_uses {
                        if warning.line < error.line {
                            patterns.push(EscalationPattern {
                                variable: var.clone(),
                                warning_line: warning.line,
                                error_line: error.line,
                                warning_context: warning.context.clone(),
                                error_context: error.context.clone(),
                            });
                        }
                    }
                }
            }
        }

        patterns
    }

    /// Find uses of .unwrap() or similar on parameters
    pub fn find_unchecked_unwraps(&self) -> Vec<UncheckedUnwrap> {
        let mut unwraps = Vec::new();

        for flow in &self.flows {
            // Check if this flow goes from parameter to unwrap
            if flow.from.kind == DefKind::Parameter {
                for transform in &flow.transformations {
                    if matches!(
                        transform.kind,
                        TransformKind::Unwrap | TransformKind::Expect
                    ) {
                        unwraps.push(UncheckedUnwrap {
                            parameter: flow.variable.clone(),
                            unwrap_line: transform.line,
                            param_line: flow.from.line,
                        });
                    }
                }
            }
        }

        unwraps
    }

    /// Get statistics about the DFG
    pub fn stats(&self) -> DfgStats {
        let total_defs: usize = self.definitions.values().map(|v| v.len()).sum();
        let total_usages: usize = self.usages.values().map(|v| v.len()).sum();

        DfgStats {
            variables: self.definitions.len(),
            definitions: total_defs,
            usages: total_usages,
            flows: self.flows.len(),
            parameters: self.parameters.len(),
        }
    }
}

/// Error escalation pattern (warning treated as error)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationPattern {
    pub variable: String,
    pub warning_line: u32,
    pub error_line: u32,
    pub warning_context: Option<String>,
    pub error_context: Option<String>,
}

/// Unchecked unwrap on a parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UncheckedUnwrap {
    pub parameter: String,
    pub param_line: u32,
    pub unwrap_line: u32,
}

/// DFG statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DfgStats {
    pub variables: usize,
    pub definitions: usize,
    pub usages: usize,
    pub flows: usize,
    pub parameters: usize,
}

// ============================================================================
// DFG Builders
// ============================================================================

/// Build a DFG from a Rust function using syn
pub fn build_dfg_rust(item_fn: &syn::ItemFn, path: &Path) -> DataFlowGraph {
    let name = item_fn.sig.ident.to_string();
    let mut dfg = DataFlowGraph::new(name, path.to_path_buf());

    // Add parameters
    for param in item_fn.sig.inputs.iter() {
        if let syn::FnArg::Typed(pat_type) = param {
            if let syn::Pat::Ident(pat_ident) = pat_type.pat.as_ref() {
                let param_name = pat_ident.ident.to_string();
                let line = pat_ident.ident.span().start().line as u32;

                dfg.add_parameter(Parameter {
                    name: param_name,
                    param_type: Some(quote::ToTokens::to_token_stream(&pat_type.ty).to_string()),
                    line,
                    has_default: false,
                    is_optional: false,
                });
            }
        }
    }

    // Visit the function body
    let mut visitor = RustDfgVisitor::new(&mut dfg);
    visitor.visit_block(&item_fn.block);

    dfg.build_flows();
    dfg
}

struct RustDfgVisitor<'a> {
    dfg: &'a mut DataFlowGraph,
}

impl<'a> RustDfgVisitor<'a> {
    fn new(dfg: &'a mut DataFlowGraph) -> Self {
        Self { dfg }
    }

    fn visit_block(&mut self, block: &syn::Block) {
        for stmt in &block.stmts {
            self.visit_stmt(stmt);
        }
    }

    fn visit_stmt(&mut self, stmt: &syn::Stmt) {
        use syn::Stmt;

        match stmt {
            Stmt::Local(local) => self.visit_local(local),
            Stmt::Expr(expr, _) => self.visit_expr(expr, false),
            Stmt::Macro(mac) => self.visit_macro(mac),
            _ => {}
        }
    }

    fn visit_local(&mut self, local: &syn::Local) {
        use quote::ToTokens;

        let line = local.let_token.span.start().line as u32;

        // Extract pattern (variable names)
        let vars = extract_pattern_vars(&local.pat);

        for var in vars {
            self.dfg.add_definition(DefSite {
                variable: var,
                line,
                column: None,
                kind: DefKind::Declaration,
                value_expr: local
                    .init
                    .as_ref()
                    .map(|i| i.expr.to_token_stream().to_string()),
                is_conditional: false,
            });
        }

        // Visit the initializer for usages
        if let Some(init) = &local.init {
            self.visit_expr(&init.expr, false);
        }
    }

    fn visit_expr(&mut self, expr: &syn::Expr, in_condition: bool) {
        use quote::ToTokens;
        use syn::Expr;

        match expr {
            Expr::Path(path) => {
                // Variable usage
                let name = path.to_token_stream().to_string();
                if !name.contains("::") {
                    // Likely a local variable
                    let line = path
                        .path
                        .segments
                        .first()
                        .map(|s| s.ident.span().start().line as u32)
                        .unwrap_or(0);

                    self.dfg.add_usage(UseSite {
                        variable: name,
                        line,
                        column: None,
                        kind: if in_condition {
                            UseKind::Condition
                        } else {
                            UseKind::Read
                        },
                        context: None,
                    });
                }
            }
            Expr::Assign(assign) => {
                // Assignment
                let line = assign.eq_token.span.start().line as u32;
                let left = assign.left.to_token_stream().to_string();

                self.dfg.add_definition(DefSite {
                    variable: left,
                    line,
                    column: None,
                    kind: DefKind::Assignment,
                    value_expr: Some(assign.right.to_token_stream().to_string()),
                    is_conditional: false,
                });

                self.visit_expr(&assign.right, false);
            }
            Expr::Binary(binary) => {
                self.visit_expr(&binary.left, in_condition);
                self.visit_expr(&binary.right, in_condition);
            }
            Expr::Call(call) => {
                self.visit_expr(&call.func, false);
                for arg in &call.args {
                    self.visit_expr(arg, false);
                }
            }
            Expr::MethodCall(method) => {
                let method_name = method.method.to_string();
                self.visit_expr(&method.receiver, false);

                // Track unwrap/expect calls
                if method_name == "unwrap" || method_name == "expect" {
                    // This is tracked in flow analysis
                }

                for arg in &method.args {
                    self.visit_expr(arg, false);
                }
            }
            Expr::If(if_expr) => {
                self.visit_expr(&if_expr.cond, true);
                self.visit_block(&if_expr.then_branch);
                if let Some((_, else_branch)) = &if_expr.else_branch {
                    self.visit_expr(else_branch, false);
                }
            }
            Expr::Match(match_expr) => {
                self.visit_expr(&match_expr.expr, true);
                for arm in &match_expr.arms {
                    self.visit_expr(&arm.body, false);
                }
            }
            Expr::Block(block) => {
                self.visit_block(&block.block);
            }
            Expr::Return(ret) => {
                if let Some(expr) = &ret.expr {
                    let line = ret.return_token.span.start().line as u32;
                    let vars = extract_expr_vars(expr);
                    for var in vars {
                        self.dfg.add_usage(UseSite {
                            variable: var,
                            line,
                            column: None,
                            kind: UseKind::Return,
                            context: None,
                        });
                    }
                    self.visit_expr(expr, false);
                }
            }
            _ => {}
        }
    }

    fn visit_macro(&mut self, stmt_macro: &syn::StmtMacro) {
        use quote::ToTokens;

        let mac_path = stmt_macro.mac.path.to_token_stream().to_string();
        let line = stmt_macro.mac.path.span().start().line as u32;

        // Check for assert macros
        if mac_path.starts_with("assert") || mac_path.starts_with("debug_assert") {
            // Variables in assertions are condition usages
            let tokens = stmt_macro.mac.tokens.to_string();
            // Simple heuristic: find identifiers in the assertion
            for word in tokens.split(|c: char| !c.is_alphanumeric() && c != '_') {
                if !word.is_empty()
                    && word
                        .chars()
                        .next()
                        .map(|c| c.is_alphabetic())
                        .unwrap_or(false)
                {
                    self.dfg.add_usage(UseSite {
                        variable: word.to_string(),
                        line,
                        column: None,
                        kind: UseKind::Assertion,
                        context: Some(mac_path.clone()),
                    });
                }
            }
        }
    }
}

fn extract_pattern_vars(pat: &syn::Pat) -> Vec<String> {
    use syn::Pat;

    let mut vars = Vec::new();
    match pat {
        Pat::Ident(ident) => {
            vars.push(ident.ident.to_string());
        }
        Pat::Tuple(tuple) => {
            for elem in &tuple.elems {
                vars.extend(extract_pattern_vars(elem));
            }
        }
        Pat::Struct(struct_pat) => {
            for field in &struct_pat.fields {
                vars.extend(extract_pattern_vars(&field.pat));
            }
        }
        Pat::TupleStruct(tuple_struct) => {
            for elem in &tuple_struct.elems {
                vars.extend(extract_pattern_vars(elem));
            }
        }
        _ => {}
    }
    vars
}

fn extract_expr_vars(expr: &syn::Expr) -> Vec<String> {
    use quote::ToTokens;
    use syn::Expr;

    let mut vars = Vec::new();
    match expr {
        Expr::Path(path) => {
            let name = path.to_token_stream().to_string();
            if !name.contains("::") {
                vars.push(name);
            }
        }
        Expr::Field(field) => {
            vars.extend(extract_expr_vars(&field.base));
        }
        Expr::Binary(binary) => {
            vars.extend(extract_expr_vars(&binary.left));
            vars.extend(extract_expr_vars(&binary.right));
        }
        Expr::Call(call) => {
            for arg in &call.args {
                vars.extend(extract_expr_vars(arg));
            }
        }
        Expr::MethodCall(method) => {
            vars.extend(extract_expr_vars(&method.receiver));
            for arg in &method.args {
                vars.extend(extract_expr_vars(arg));
            }
        }
        _ => {}
    }
    vars
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dfg_basic() {
        let mut dfg = DataFlowGraph::new("test".to_string(), PathBuf::from("test.rs"));

        dfg.add_parameter(Parameter {
            name: "x".to_string(),
            param_type: Some("i32".to_string()),
            line: 1,
            has_default: false,
            is_optional: false,
        });

        dfg.add_usage(UseSite {
            variable: "x".to_string(),
            line: 2,
            column: None,
            kind: UseKind::Read,
            context: None,
        });

        dfg.build_flows();

        assert_eq!(dfg.parameters.len(), 1);
        assert_eq!(dfg.flows.len(), 1);
    }

    #[test]
    fn test_dfg_escalation() {
        let mut dfg = DataFlowGraph::new("test".to_string(), PathBuf::from("test.rs"));

        dfg.add_definition(DefSite {
            variable: "count".to_string(),
            line: 1,
            column: None,
            kind: DefKind::Declaration,
            value_expr: Some("0".to_string()),
            is_conditional: false,
        });

        dfg.add_usage(UseSite {
            variable: "count".to_string(),
            line: 2,
            column: None,
            kind: UseKind::Read,
            context: Some("warning_count".to_string()),
        });

        dfg.add_usage(UseSite {
            variable: "count".to_string(),
            line: 3,
            column: None,
            kind: UseKind::Read,
            context: Some("error_count".to_string()),
        });

        let escalations = dfg.find_error_escalation();
        assert_eq!(escalations.len(), 1);
    }
}
