//! Control Flow Graph (CFG) construction and analysis.
//!
//! This module builds control flow graphs from Rust, TypeScript, and Python code
//! to enable static analysis of execution paths, exit conditions, and logic flow.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use syn::spanned::Spanned;

/// Unique identifier for a CFG node
pub type NodeId = u32;

/// Control Flow Graph for a single function/method
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlFlowGraph {
    /// Function/method name
    pub name: String,
    /// File path
    pub path: PathBuf,
    /// Starting line of the function
    pub start_line: u32,
    /// All nodes (basic blocks) in the graph
    pub nodes: Vec<CfgNode>,
    /// All edges (control flow transitions)
    pub edges: Vec<CfgEdge>,
    /// Entry node ID
    pub entry: NodeId,
    /// Exit node IDs (return, panic, exit points)
    pub exits: Vec<NodeId>,
    /// Language of the source
    pub language: CfgLanguage,
}

/// Language of the CFG source
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CfgLanguage {
    Rust,
    TypeScript,
    JavaScript,
    Python,
}

/// A node (basic block) in the control flow graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfgNode {
    /// Unique ID within the CFG
    pub id: NodeId,
    /// Kind of node
    pub kind: NodeKind,
    /// Statements in this basic block
    pub statements: Vec<Statement>,
    /// Source line range (start, end)
    pub source_range: (u32, u32),
    /// Whether this node is reachable from entry
    pub reachable: bool,
}

/// Kind of CFG node
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    /// Entry point of the function
    Entry,
    /// Regular basic block (sequence of statements)
    Block,
    /// If/else branch point
    If,
    /// Match/switch branch point
    Match,
    /// Loop header (for, while, loop)
    Loop,
    /// Return statement
    Return,
    /// Early exit (process::exit, sys.exit, etc.)
    Exit,
    /// Panic/throw/raise
    Panic,
    /// Break statement
    Break,
    /// Continue statement
    Continue,
    /// Function call that may not return (e.g., diverging functions)
    Diverge,
}

/// A statement within a basic block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Statement {
    /// Line number
    pub line: u32,
    /// Kind of statement
    pub kind: StatementKind,
    /// Relevant identifiers (variables read/written)
    pub identifiers: Vec<String>,
    /// Source text snippet
    pub text: String,
}

/// Kind of statement
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StatementKind {
    /// Variable declaration/binding
    Declaration,
    /// Assignment
    Assignment,
    /// Compound assignment (+=, -=, etc.)
    CompoundAssignment,
    /// Function call
    Call,
    /// Return statement
    Return,
    /// Expression statement
    Expression,
    /// Assertion (assert!, debug_assert!, etc.)
    Assertion,
    /// Other
    Other,
}

/// An edge in the control flow graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfgEdge {
    /// Source node
    pub from: NodeId,
    /// Target node
    pub to: NodeId,
    /// Edge kind
    pub kind: EdgeKind,
    /// Condition expression (for conditional branches)
    pub condition: Option<String>,
}

/// Kind of CFG edge
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeKind {
    /// Unconditional fallthrough
    Fallthrough,
    /// True branch of conditional
    TrueBranch,
    /// False branch of conditional
    FalseBranch,
    /// Match arm
    MatchArm,
    /// Loop back edge
    LoopBack,
    /// Loop exit
    LoopExit,
    /// Exception/error path
    Exception,
}

/// A path through the CFG to an exit point
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExitPath {
    /// Sequence of node IDs from entry to exit
    pub nodes: Vec<NodeId>,
    /// Conditions that must be true for this path
    pub conditions: Vec<PathCondition>,
    /// The exit node
    pub exit_node: NodeId,
    /// Kind of exit (return, panic, exit)
    pub exit_kind: NodeKind,
}

/// A condition that must hold for a path
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathCondition {
    /// The condition expression
    pub expression: String,
    /// Whether it must be true or false
    pub must_be_true: bool,
    /// Line where the condition is checked
    pub line: u32,
}

impl ControlFlowGraph {
    /// Create a new empty CFG
    pub fn new(name: String, path: PathBuf, start_line: u32, language: CfgLanguage) -> Self {
        Self {
            name,
            path,
            start_line,
            nodes: Vec::new(),
            edges: Vec::new(),
            entry: 0,
            exits: Vec::new(),
            language,
        }
    }

    /// Add a node to the CFG
    pub fn add_node(&mut self, kind: NodeKind, source_range: (u32, u32)) -> NodeId {
        let id = self.nodes.len() as NodeId;
        self.nodes.push(CfgNode {
            id,
            kind,
            statements: Vec::new(),
            source_range,
            reachable: false,
        });
        id
    }

    /// Add an edge between nodes
    pub fn add_edge(&mut self, from: NodeId, to: NodeId, kind: EdgeKind, condition: Option<String>) {
        self.edges.push(CfgEdge {
            from,
            to,
            kind,
            condition,
        });
    }

    /// Add a statement to a node
    pub fn add_statement(&mut self, node_id: NodeId, statement: Statement) {
        if let Some(node) = self.nodes.get_mut(node_id as usize) {
            node.statements.push(statement);
        }
    }

    /// Mark exit nodes
    pub fn mark_exits(&mut self) {
        self.exits.clear();
        for node in &self.nodes {
            match node.kind {
                NodeKind::Return | NodeKind::Exit | NodeKind::Panic | NodeKind::Diverge => {
                    self.exits.push(node.id);
                }
                _ => {}
            }
        }
    }

    /// Compute reachability from entry node
    pub fn compute_reachability(&mut self) {
        // Reset all nodes to unreachable
        for node in &mut self.nodes {
            node.reachable = false;
        }

        // BFS from entry
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(self.entry);

        while let Some(node_id) = queue.pop_front() {
            if visited.contains(&node_id) {
                continue;
            }
            visited.insert(node_id);

            if let Some(node) = self.nodes.get_mut(node_id as usize) {
                node.reachable = true;
            }

            // Add successors
            for edge in &self.edges {
                if edge.from == node_id && !visited.contains(&edge.to) {
                    queue.push_back(edge.to);
                }
            }
        }
    }

    /// Get unreachable nodes (dead code candidates)
    pub fn unreachable_nodes(&self) -> Vec<&CfgNode> {
        self.nodes.iter().filter(|n| !n.reachable).collect()
    }

    /// Get all paths from entry to exit nodes
    pub fn all_exit_paths(&self) -> Vec<ExitPath> {
        let mut paths = Vec::new();
        let mut current_path = Vec::new();
        let mut conditions = Vec::new();
        let mut visited = HashSet::new();

        self.dfs_exit_paths(
            self.entry,
            &mut current_path,
            &mut conditions,
            &mut visited,
            &mut paths,
        );

        paths
    }

    fn dfs_exit_paths(
        &self,
        node_id: NodeId,
        current_path: &mut Vec<NodeId>,
        conditions: &mut Vec<PathCondition>,
        visited: &mut HashSet<NodeId>,
        paths: &mut Vec<ExitPath>,
    ) {
        if visited.contains(&node_id) {
            return; // Cycle detected, skip
        }

        current_path.push(node_id);
        visited.insert(node_id);

        let node = match self.nodes.get(node_id as usize) {
            Some(n) => n,
            None => {
                current_path.pop();
                visited.remove(&node_id);
                return;
            }
        };

        // Check if this is an exit node
        match node.kind {
            NodeKind::Return | NodeKind::Exit | NodeKind::Panic | NodeKind::Diverge => {
                paths.push(ExitPath {
                    nodes: current_path.clone(),
                    conditions: conditions.clone(),
                    exit_node: node_id,
                    exit_kind: node.kind.clone(),
                });
            }
            _ => {
                // Continue DFS through successors
                for edge in &self.edges {
                    if edge.from == node_id {
                        // Add condition to path if present
                        if let Some(ref cond) = edge.condition {
                            conditions.push(PathCondition {
                                expression: cond.clone(),
                                must_be_true: edge.kind == EdgeKind::TrueBranch,
                                line: node.source_range.0,
                            });
                        }

                        self.dfs_exit_paths(edge.to, current_path, conditions, visited, paths);

                        if edge.condition.is_some() {
                            conditions.pop();
                        }
                    }
                }
            }
        }

        current_path.pop();
        visited.remove(&node_id);
    }

    /// Find paths that lead to a specific exit kind
    pub fn paths_to_exit_kind(&self, exit_kind: &NodeKind) -> Vec<ExitPath> {
        self.all_exit_paths()
            .into_iter()
            .filter(|p| &p.exit_kind == exit_kind)
            .collect()
    }

    /// Get successors of a node
    pub fn successors(&self, node_id: NodeId) -> Vec<NodeId> {
        self.edges
            .iter()
            .filter(|e| e.from == node_id)
            .map(|e| e.to)
            .collect()
    }

    /// Get predecessors of a node
    pub fn predecessors(&self, node_id: NodeId) -> Vec<NodeId> {
        self.edges
            .iter()
            .filter(|e| e.to == node_id)
            .map(|e| e.from)
            .collect()
    }

    /// Export CFG as Mermaid diagram
    pub fn to_mermaid(&self) -> String {
        let mut lines = vec![String::from("flowchart TD")];

        // Add nodes
        for node in &self.nodes {
            let label = match &node.kind {
                NodeKind::Entry => "Entry".to_string(),
                NodeKind::Block => format!("Block L{}-{}", node.source_range.0, node.source_range.1),
                NodeKind::If => format!("If L{}", node.source_range.0),
                NodeKind::Match => format!("Match L{}", node.source_range.0),
                NodeKind::Loop => format!("Loop L{}", node.source_range.0),
                NodeKind::Return => format!("Return L{}", node.source_range.0),
                NodeKind::Exit => format!("Exit L{}", node.source_range.0),
                NodeKind::Panic => format!("Panic L{}", node.source_range.0),
                NodeKind::Break => format!("Break L{}", node.source_range.0),
                NodeKind::Continue => format!("Continue L{}", node.source_range.0),
                NodeKind::Diverge => format!("Diverge L{}", node.source_range.0),
            };

            let shape = match &node.kind {
                NodeKind::If | NodeKind::Match => format!("    N{}{{{{ {} }}}}", node.id, label),
                NodeKind::Exit | NodeKind::Panic => format!("    N{}[[ {} ]]", node.id, label),
                NodeKind::Return => format!("    N{}([[ {} ]])", node.id, label),
                _ => format!("    N{}[{}]", node.id, label),
            };
            lines.push(shape);
        }

        // Add edges
        for edge in &self.edges {
            let label = match &edge.condition {
                Some(cond) => format!(" -->|\"{}\"| ", cond),
                None => " --> ".to_string(),
            };
            lines.push(format!("    N{}{} N{}", edge.from, label, edge.to));
        }

        lines.join("\n")
    }

    /// Export CFG stats
    pub fn stats(&self) -> CfgStats {
        let unreachable = self.nodes.iter().filter(|n| !n.reachable).count();
        let exit_paths = self.all_exit_paths().len();

        CfgStats {
            nodes: self.nodes.len(),
            edges: self.edges.len(),
            exits: self.exits.len(),
            unreachable_nodes: unreachable,
            exit_paths,
        }
    }
}

fn add_flow_node(
    cfg: &mut ControlFlowGraph,
    current_node: NodeId,
    kind: NodeKind,
    line: u32,
) -> NodeId {
    let node = cfg.add_node(kind, (line, line));
    cfg.add_edge(current_node, node, EdgeKind::Fallthrough, None);
    node
}

fn add_implicit_return_if_needed(cfg: &mut ControlFlowGraph, current_node: NodeId) {
    if let Some(node) = cfg.nodes.get(current_node as usize) {
        match node.kind {
            NodeKind::Return | NodeKind::Exit | NodeKind::Panic | NodeKind::Diverge => {}
            _ => {
                let ret = cfg.add_node(NodeKind::Return, node.source_range);
                cfg.add_edge(current_node, ret, EdgeKind::Fallthrough, None);
            }
        }
    }
}

fn handle_return(cfg: &mut ControlFlowGraph, current_node: &mut NodeId, line: u32) {
    *current_node = add_flow_node(cfg, *current_node, NodeKind::Return, line);
}

fn handle_panic(cfg: &mut ControlFlowGraph, current_node: &mut NodeId, line: u32) {
    *current_node = add_flow_node(cfg, *current_node, NodeKind::Panic, line);
}

fn handle_break(
    cfg: &mut ControlFlowGraph,
    current_node: &mut NodeId,
    loop_stack: &[(NodeId, NodeId)],
    line: u32,
) {
    let break_node = add_flow_node(cfg, *current_node, NodeKind::Break, line);
    if let Some((_header, exit)) = loop_stack.last() {
        cfg.add_edge(break_node, *exit, EdgeKind::LoopExit, None);
    }
    *current_node = break_node;
}

fn handle_continue(
    cfg: &mut ControlFlowGraph,
    current_node: &mut NodeId,
    loop_stack: &[(NodeId, NodeId)],
    line: u32,
) {
    let continue_node = add_flow_node(cfg, *current_node, NodeKind::Continue, line);
    if let Some((header, _exit)) = loop_stack.last() {
        cfg.add_edge(continue_node, *header, EdgeKind::LoopBack, None);
    }
    *current_node = continue_node;
}

/// Statistics about a CFG
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfgStats {
    pub nodes: usize,
    pub edges: usize,
    pub exits: usize,
    pub unreachable_nodes: usize,
    pub exit_paths: usize,
}

/// Collection of CFGs for a project
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectCfg {
    /// CFGs indexed by (file_path, function_name)
    pub cfgs: HashMap<(PathBuf, String), ControlFlowGraph>,
}

impl ProjectCfg {
    pub fn add(&mut self, cfg: ControlFlowGraph) {
        let key = (cfg.path.clone(), cfg.name.clone());
        self.cfgs.insert(key, cfg);
    }

    pub fn get(&self, path: &Path, name: &str) -> Option<&ControlFlowGraph> {
        self.cfgs.get(&(path.to_path_buf(), name.to_string()))
    }

    pub fn iter(&self) -> impl Iterator<Item = &ControlFlowGraph> {
        self.cfgs.values()
    }

    pub fn stats(&self) -> ProjectCfgStats {
        let total_nodes: usize = self.cfgs.values().map(|c| c.nodes.len()).sum();
        let total_edges: usize = self.cfgs.values().map(|c| c.edges.len()).sum();
        let total_unreachable: usize = self
            .cfgs
            .values()
            .map(|c| c.nodes.iter().filter(|n| !n.reachable).count())
            .sum();

        ProjectCfgStats {
            functions: self.cfgs.len(),
            total_nodes,
            total_edges,
            total_unreachable,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectCfgStats {
    pub functions: usize,
    pub total_nodes: usize,
    pub total_edges: usize,
    pub total_unreachable: usize,
}

// ============================================================================
// Rust CFG Builder (using syn)
// ============================================================================

/// Build a CFG from a Rust function using syn
pub fn build_cfg_rust(item_fn: &syn::ItemFn, path: &Path) -> ControlFlowGraph {
    let name = item_fn.sig.ident.to_string();
    let start_line = item_fn.sig.ident.span().start().line as u32;
    
    let mut cfg = ControlFlowGraph::new(
        name,
        path.to_path_buf(),
        start_line,
        CfgLanguage::Rust,
    );

    let mut builder = RustCfgBuilder::new(&mut cfg);
    builder.visit_block(&item_fn.block);
    builder.finalize();

    cfg.mark_exits();
    cfg.compute_reachability();
    cfg
}

/// Build CFG from a Rust method in an impl block
pub fn build_cfg_rust_method(method: &syn::ImplItemFn, path: &Path, self_ty: &str) -> ControlFlowGraph {
    let fn_name = method.sig.ident.to_string();
    let name = format!("{}::{}", self_ty, fn_name);
    let start_line = method.sig.ident.span().start().line as u32;

    let mut cfg = ControlFlowGraph::new(
        name,
        path.to_path_buf(),
        start_line,
        CfgLanguage::Rust,
    );

    let mut builder = RustCfgBuilder::new(&mut cfg);
    builder.visit_block(&method.block);
    builder.finalize();

    cfg.mark_exits();
    cfg.compute_reachability();
    cfg
}

struct RustCfgBuilder<'a> {
    cfg: &'a mut ControlFlowGraph,
    current_node: NodeId,
    /// Stack of loop headers for break/continue
    loop_stack: Vec<(NodeId, NodeId)>, // (header, exit)
}

impl<'a> RustCfgBuilder<'a> {
    fn new(cfg: &'a mut ControlFlowGraph) -> Self {
        // Create entry node
        let entry = cfg.add_node(NodeKind::Entry, (0, 0));
        cfg.entry = entry;

        // Create initial block
        let block = cfg.add_node(NodeKind::Block, (0, 0));
        cfg.add_edge(entry, block, EdgeKind::Fallthrough, None);

        Self {
            cfg,
            current_node: block,
            loop_stack: Vec::new(),
        }
    }

    fn finalize(&mut self) {
        add_implicit_return_if_needed(self.cfg, self.current_node);
    }

    fn visit_block(&mut self, block: &syn::Block) {
        use syn::Stmt;
        
        for stmt in &block.stmts {
            match stmt {
                Stmt::Local(local) => self.visit_local(local),
                Stmt::Item(_) => {} // Nested items, skip
                Stmt::Expr(expr, _semi) => self.visit_expr(expr),
                Stmt::Macro(mac) => self.visit_macro(mac),
            }
        }
    }

    fn visit_local(&mut self, local: &syn::Local) {
        use quote::ToTokens;
        
        let line = local.let_token.span.start().line as u32;
        
        // Extract pattern (variable name)
        let pat_str = local.pat.to_token_stream().to_string();
        let mut identifiers = vec![pat_str.clone()];
        
        // Extract identifiers from init expression
        if let Some(init) = &local.init {
            self.extract_identifiers(&init.expr, &mut identifiers);
        }

        let stmt = Statement {
            line,
            kind: StatementKind::Declaration,
            identifiers,
            text: format!("let {} = ...", pat_str),
        };

        self.cfg.add_statement(self.current_node, stmt);
        self.update_source_range(line);
    }

    fn visit_expr(&mut self, expr: &syn::Expr) {
        use syn::Expr;
        use quote::ToTokens;

        match expr {
            Expr::If(expr_if) => self.visit_if(expr_if),
            Expr::Match(expr_match) => self.visit_match(expr_match),
            Expr::Loop(expr_loop) => self.visit_loop(expr_loop),
            Expr::While(expr_while) => self.visit_while(expr_while),
            Expr::ForLoop(expr_for) => self.visit_for(expr_for),
            Expr::Return(expr_return) => self.visit_return(expr_return),
            Expr::Break(expr_break) => self.visit_break(expr_break),
            Expr::Continue(expr_continue) => self.visit_continue(expr_continue),
            Expr::Block(expr_block) => self.visit_block(&expr_block.block),
            Expr::Call(call) => {
                let line = call.func.span().start().line as u32;
                let func_name = call.func.to_token_stream().to_string();
                
                // Check for diverging/exit functions
                if is_rust_exit_call(&func_name) {
                    let exit_node = self.cfg.add_node(NodeKind::Exit, (line, line));
                    self.cfg.add_edge(self.current_node, exit_node, EdgeKind::Fallthrough, None);
                    self.current_node = exit_node;
                } else if is_rust_panic_call(&func_name) {
                    let panic_node = self.cfg.add_node(NodeKind::Panic, (line, line));
                    self.cfg.add_edge(self.current_node, panic_node, EdgeKind::Fallthrough, None);
                    self.current_node = panic_node;
                } else {
                    let mut identifiers = vec![func_name.clone()];
                    for arg in &call.args {
                        self.extract_identifiers(arg, &mut identifiers);
                    }

                    let stmt = Statement {
                        line,
                        kind: StatementKind::Call,
                        identifiers,
                        text: format!("{}(...)", func_name),
                    };
                    self.cfg.add_statement(self.current_node, stmt);
                    self.update_source_range(line);
                }
            }
            Expr::MethodCall(method) => {
                let line = method.method.span().start().line as u32;
                let method_name = method.method.to_string();
                
                let mut identifiers = vec![method_name.clone()];
                self.extract_identifiers(&method.receiver, &mut identifiers);
                for arg in &method.args {
                    self.extract_identifiers(arg, &mut identifiers);
                }

                let stmt = Statement {
                    line,
                    kind: StatementKind::Call,
                    identifiers,
                    text: format!(".{}(...)", method_name),
                };
                self.cfg.add_statement(self.current_node, stmt);
                self.update_source_range(line);
            }
            Expr::Assign(assign) => {
                let line = assign.eq_token.span.start().line as u32;
                let mut identifiers = Vec::new();
                self.extract_identifiers(&assign.left, &mut identifiers);
                self.extract_identifiers(&assign.right, &mut identifiers);

                let stmt = Statement {
                    line,
                    kind: StatementKind::Assignment,
                    identifiers,
                    text: "assignment".into(),
                };
                self.cfg.add_statement(self.current_node, stmt);
                self.update_source_range(line);
            }
            _ => {
                // Other expressions - add as generic statement
                let line = get_expr_start_line(expr);
                if line > 0 {
                    let stmt = Statement {
                        line,
                        kind: StatementKind::Expression,
                        identifiers: Vec::new(),
                        text: "expr".into(),
                    };
                    self.cfg.add_statement(self.current_node, stmt);
                    self.update_source_range(line);
                }
            }
        }
    }

    fn visit_if(&mut self, expr_if: &syn::ExprIf) {
        use quote::ToTokens;
        
        let line = expr_if.if_token.span.start().line as u32;
        let cond_str = expr_if.cond.to_token_stream().to_string();

        // Create If node
        let if_node = self.cfg.add_node(NodeKind::If, (line, line));
        self.cfg.add_edge(self.current_node, if_node, EdgeKind::Fallthrough, None);

        // Create merge node (may become unreachable if both branches exit)
        let end_line = get_block_end_line(&expr_if.then_branch);
        let merge_node = self.cfg.add_node(NodeKind::Block, (end_line, end_line));

        // Track if any branch reaches the merge node
        let mut true_reaches_merge = false;
        let mut false_reaches_merge = false;

        // True branch
        let true_block = self.cfg.add_node(NodeKind::Block, (line + 1, line + 1));
        self.cfg.add_edge(if_node, true_block, EdgeKind::TrueBranch, Some(cond_str.clone()));
        
        self.current_node = true_block;
        self.visit_block(&expr_if.then_branch);
        
        // Connect to merge if not already exited
        if !self.is_exit_node(self.current_node) {
            self.cfg.add_edge(self.current_node, merge_node, EdgeKind::Fallthrough, None);
            true_reaches_merge = true;
        }

        // False branch
        if let Some((_, else_branch)) = &expr_if.else_branch {
            let false_block = self.cfg.add_node(NodeKind::Block, (line + 1, line + 1));
            self.cfg.add_edge(if_node, false_block, EdgeKind::FalseBranch, Some(cond_str));
            
            self.current_node = false_block;
            self.visit_expr(else_branch);
            
            if !self.is_exit_node(self.current_node) {
                self.cfg.add_edge(self.current_node, merge_node, EdgeKind::Fallthrough, None);
                false_reaches_merge = true;
            }
        } else {
            // No else - false branch goes directly to merge
            self.cfg.add_edge(if_node, merge_node, EdgeKind::FalseBranch, Some(cond_str));
            false_reaches_merge = true;
        }

        // Only continue from merge if at least one branch reaches it
        // Otherwise, both branches exit and merge is unreachable (but we don't flag it as dead code)
        if true_reaches_merge || false_reaches_merge {
            self.current_node = merge_node;
        }
        // If neither branch reaches merge, current_node stays at last exit node,
        // which means subsequent code will be correctly marked as unreachable
    }

    fn visit_match(&mut self, expr_match: &syn::ExprMatch) {
        use quote::ToTokens;
        
        let line = expr_match.match_token.span.start().line as u32;
        let _scrutinee = expr_match.expr.to_token_stream().to_string();

        let match_node = self.cfg.add_node(NodeKind::Match, (line, line));
        self.cfg.add_edge(self.current_node, match_node, EdgeKind::Fallthrough, None);

        // Create merge node
        let merge_node = self.cfg.add_node(NodeKind::Block, (line, line));

        for arm in &expr_match.arms {
            let pat_str = arm.pat.to_token_stream().to_string();
            let arm_line = arm.pat.span().start().line as u32;
            
            let arm_block = self.cfg.add_node(NodeKind::Block, (arm_line, arm_line));
            self.cfg.add_edge(match_node, arm_block, EdgeKind::MatchArm, Some(pat_str));
            
            self.current_node = arm_block;
            self.visit_expr(&arm.body);
            
            if !self.is_exit_node(self.current_node) {
                self.cfg.add_edge(self.current_node, merge_node, EdgeKind::Fallthrough, None);
            }
        }

        self.current_node = merge_node;
    }

    fn visit_loop(&mut self, expr_loop: &syn::ExprLoop) {
        let line = expr_loop.loop_token.span.start().line as u32;
        
        let header = self.cfg.add_node(NodeKind::Loop, (line, line));
        self.cfg.add_edge(self.current_node, header, EdgeKind::Fallthrough, None);

        let body = self.cfg.add_node(NodeKind::Block, (line + 1, line + 1));
        self.cfg.add_edge(header, body, EdgeKind::Fallthrough, None);

        let exit = self.cfg.add_node(NodeKind::Block, (line, line));

        self.loop_stack.push((header, exit));
        self.current_node = body;
        self.visit_block(&expr_loop.body);
        
        if !self.is_exit_node(self.current_node) {
            self.cfg.add_edge(self.current_node, header, EdgeKind::LoopBack, None);
        }
        
        self.loop_stack.pop();
        self.current_node = exit;
    }

    fn visit_while(&mut self, expr_while: &syn::ExprWhile) {
        use quote::ToTokens;
        
        let line = expr_while.while_token.span.start().line as u32;
        let cond = expr_while.cond.to_token_stream().to_string();
        
        let header = self.cfg.add_node(NodeKind::Loop, (line, line));
        self.cfg.add_edge(self.current_node, header, EdgeKind::Fallthrough, None);

        let body = self.cfg.add_node(NodeKind::Block, (line + 1, line + 1));
        self.cfg.add_edge(header, body, EdgeKind::TrueBranch, Some(cond.clone()));

        let exit = self.cfg.add_node(NodeKind::Block, (line, line));
        self.cfg.add_edge(header, exit, EdgeKind::LoopExit, Some(format!("!{}", cond)));

        self.loop_stack.push((header, exit));
        self.current_node = body;
        self.visit_block(&expr_while.body);
        
        if !self.is_exit_node(self.current_node) {
            self.cfg.add_edge(self.current_node, header, EdgeKind::LoopBack, None);
        }
        
        self.loop_stack.pop();
        self.current_node = exit;
    }

    fn visit_for(&mut self, expr_for: &syn::ExprForLoop) {
        use quote::ToTokens;
        
        let line = expr_for.for_token.span.start().line as u32;
        let pat = expr_for.pat.to_token_stream().to_string();
        let iter = expr_for.expr.to_token_stream().to_string();
        
        let header = self.cfg.add_node(NodeKind::Loop, (line, line));
        self.cfg.add_edge(self.current_node, header, EdgeKind::Fallthrough, None);

        let body = self.cfg.add_node(NodeKind::Block, (line + 1, line + 1));
        self.cfg.add_edge(header, body, EdgeKind::TrueBranch, Some(format!("{} in {}", pat, iter)));

        let exit = self.cfg.add_node(NodeKind::Block, (line, line));
        self.cfg.add_edge(header, exit, EdgeKind::LoopExit, None);

        self.loop_stack.push((header, exit));
        self.current_node = body;
        self.visit_block(&expr_for.body);
        
        if !self.is_exit_node(self.current_node) {
            self.cfg.add_edge(self.current_node, header, EdgeKind::LoopBack, None);
        }
        
        self.loop_stack.pop();
        self.current_node = exit;
    }

    fn visit_return(&mut self, expr_return: &syn::ExprReturn) {
        let line = expr_return.return_token.span.start().line as u32;
        handle_return(self.cfg, &mut self.current_node, line);
    }

    fn visit_break(&mut self, expr_break: &syn::ExprBreak) {
        let line = expr_break.break_token.span.start().line as u32;
        handle_break(self.cfg, &mut self.current_node, &self.loop_stack, line);
    }

    fn visit_continue(&mut self, expr_continue: &syn::ExprContinue) {
        let line = expr_continue.continue_token.span.start().line as u32;
        handle_continue(self.cfg, &mut self.current_node, &self.loop_stack, line);
    }

    fn visit_macro(&mut self, stmt_macro: &syn::StmtMacro) {
        use quote::ToTokens;
        
        let mac_path = stmt_macro.mac.path.to_token_stream().to_string();
        let line = stmt_macro.mac.path.span().start().line as u32;
        
        if is_rust_panic_macro(&mac_path) {
            let panic_node = self.cfg.add_node(NodeKind::Panic, (line, line));
            self.cfg.add_edge(self.current_node, panic_node, EdgeKind::Fallthrough, None);
            self.current_node = panic_node;
        } else if is_rust_assert_macro(&mac_path) {
            let stmt = Statement {
                line,
                kind: StatementKind::Assertion,
                identifiers: vec![mac_path],
                text: "assertion".into(),
            };
            self.cfg.add_statement(self.current_node, stmt);
            self.update_source_range(line);
        } else {
            let stmt = Statement {
                line,
                kind: StatementKind::Call,
                identifiers: vec![mac_path],
                text: "macro call".into(),
            };
            self.cfg.add_statement(self.current_node, stmt);
            self.update_source_range(line);
        }
    }

    fn is_exit_node(&self, node_id: NodeId) -> bool {
        if let Some(node) = self.cfg.nodes.get(node_id as usize) {
            matches!(
                node.kind,
                NodeKind::Return | NodeKind::Exit | NodeKind::Panic | NodeKind::Break | NodeKind::Continue
            )
        } else {
            false
        }
    }

    fn update_source_range(&mut self, line: u32) {
        if let Some(node) = self.cfg.nodes.get_mut(self.current_node as usize) {
            if node.source_range.0 == 0 {
                node.source_range.0 = line;
            }
            node.source_range.1 = line;
        }
    }

    fn extract_identifiers(&self, expr: &syn::Expr, identifiers: &mut Vec<String>) {
        use syn::Expr;
        use quote::ToTokens;

        match expr {
            Expr::Path(path) => {
                identifiers.push(path.to_token_stream().to_string());
            }
            Expr::Field(field) => {
                self.extract_identifiers(&field.base, identifiers);
            }
            Expr::Call(call) => {
                self.extract_identifiers(&call.func, identifiers);
                for arg in &call.args {
                    self.extract_identifiers(arg, identifiers);
                }
            }
            Expr::MethodCall(method) => {
                self.extract_identifiers(&method.receiver, identifiers);
            }
            Expr::Binary(binary) => {
                self.extract_identifiers(&binary.left, identifiers);
                self.extract_identifiers(&binary.right, identifiers);
            }
            Expr::Unary(unary) => {
                self.extract_identifiers(&unary.expr, identifiers);
            }
            Expr::Reference(reference) => {
                self.extract_identifiers(&reference.expr, identifiers);
            }
            _ => {}
        }
    }
}

fn is_rust_exit_call(func: &str) -> bool {
    func.contains("std::process::exit")
        || func.contains("process::exit")
        || func == "exit"
}

fn is_rust_panic_call(func: &str) -> bool {
    func.contains("panic")
        || func.contains("unreachable")
        || func.contains("unimplemented")
        || func.contains("todo")
}

fn is_rust_panic_macro(mac: &str) -> bool {
    mac == "panic"
        || mac == "unreachable"
        || mac == "unimplemented"
        || mac == "todo"
}

fn is_rust_assert_macro(mac: &str) -> bool {
    mac.starts_with("assert")
        || mac.starts_with("debug_assert")
}

fn get_expr_start_line(expr: &syn::Expr) -> u32 {
    use syn::Expr;
    use proc_macro2::Span;

    fn span_start(span: Span) -> u32 {
        span.start().line as u32
    }

    match expr {
        Expr::Array(e) => span_start(e.bracket_token.span.open()),
        Expr::Assign(e) => span_start(e.eq_token.span),
        Expr::Binary(e) => get_expr_start_line(&e.left),
        Expr::Block(e) => span_start(e.block.brace_token.span.open()),
        Expr::Break(e) => span_start(e.break_token.span),
        Expr::Call(e) => get_expr_start_line(&e.func),
        Expr::Continue(e) => span_start(e.continue_token.span),
        Expr::Field(e) => get_expr_start_line(&e.base),
        Expr::ForLoop(e) => span_start(e.for_token.span),
        Expr::If(e) => span_start(e.if_token.span),
        Expr::Loop(e) => span_start(e.loop_token.span),
        Expr::Match(e) => span_start(e.match_token.span),
        Expr::MethodCall(e) => get_expr_start_line(&e.receiver),
        Expr::Path(e) => e.path.segments.first().map(|s| span_start(s.ident.span())).unwrap_or(0),
        Expr::Return(e) => span_start(e.return_token.span),
        Expr::While(e) => span_start(e.while_token.span),
        _ => 0,
    }
}

fn get_block_end_line(block: &syn::Block) -> u32 {
    block.brace_token.span.close().start().line as u32
}

// ============================================================================
// TypeScript/JavaScript CFG Builder (using tree-sitter)
// ============================================================================

/// Build a CFG from a TypeScript/JavaScript function using tree-sitter
pub fn build_cfg_ts(
    node: tree_sitter::Node,
    source: &[u8],
    path: &Path,
    is_typescript: bool,
) -> Option<ControlFlowGraph> {
    // Must be a function node
    let kind = node.kind();
    if !matches!(kind, "function_declaration" | "function" | "arrow_function" 
        | "method_definition" | "function_expression") {
        return None;
    }

    let name = extract_ts_function_name(&node, source);
    let start_line = node.start_position().row as u32 + 1;
    let language = if is_typescript {
        CfgLanguage::TypeScript
    } else {
        CfgLanguage::JavaScript
    };

    let mut cfg = ControlFlowGraph::new(name, path.to_path_buf(), start_line, language);
    
    // Find the body
    let body = node.child_by_field_name("body")?;
    
    let mut builder = TsCfgBuilder::new(&mut cfg, source);
    builder.visit_node(&body);
    builder.finalize();

    cfg.mark_exits();
    cfg.compute_reachability();
    Some(cfg)
}

fn extract_ts_function_name(node: &tree_sitter::Node, source: &[u8]) -> String {
    // Try to find name child
    if let Some(name_node) = node.child_by_field_name("name") {
        if let Ok(name) = name_node.utf8_text(source) {
            return name.to_string();
        }
    }

    // For arrow functions assigned to a variable, check parent
    // Default to anonymous
    format!("anonymous_{}", node.start_position().row)
}

struct TsCfgBuilder<'a> {
    cfg: &'a mut ControlFlowGraph,
    source: &'a [u8],
    current_node: NodeId,
    loop_stack: Vec<(NodeId, NodeId)>,
}

impl<'a> TsCfgBuilder<'a> {
    fn new(cfg: &'a mut ControlFlowGraph, source: &'a [u8]) -> Self {
        let entry = cfg.add_node(NodeKind::Entry, (0, 0));
        cfg.entry = entry;

        let block = cfg.add_node(NodeKind::Block, (0, 0));
        cfg.add_edge(entry, block, EdgeKind::Fallthrough, None);

        Self {
            cfg,
            source,
            current_node: block,
            loop_stack: Vec::new(),
        }
    }

    fn finalize(&mut self) {
        add_implicit_return_if_needed(self.cfg, self.current_node);
    }

    fn visit_node(&mut self, node: &tree_sitter::Node) {
        match node.kind() {
            "statement_block" => self.visit_block(node),
            "if_statement" => self.visit_if(node),
            "switch_statement" => self.visit_switch(node),
            "for_statement" | "for_in_statement" | "for_of_statement" => self.visit_for(node),
            "while_statement" => self.visit_while(node),
            "do_statement" => self.visit_do_while(node),
            "return_statement" => self.visit_return(node),
            "break_statement" => self.visit_break(node),
            "continue_statement" => self.visit_continue(node),
            "throw_statement" => self.visit_throw(node),
            "try_statement" => self.visit_try(node),
            "expression_statement" => self.visit_expression_statement(node),
            "variable_declaration" | "lexical_declaration" => self.visit_declaration(node),
            _ => {
                // Process children
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    self.visit_node(&child);
                }
            }
        }
    }

    fn visit_block(&mut self, node: &tree_sitter::Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.is_named() {
                self.visit_node(&child);
            }
        }
    }

    fn visit_if(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        
        let cond = node.child_by_field_name("condition")
            .and_then(|c| c.utf8_text(self.source).ok())
            .unwrap_or("")
            .to_string();

        let if_node = self.cfg.add_node(NodeKind::If, (line, line));
        self.cfg.add_edge(self.current_node, if_node, EdgeKind::Fallthrough, None);

        let end_line = node.end_position().row as u32 + 1;
        let merge_node = self.cfg.add_node(NodeKind::Block, (end_line, end_line));

        // True branch (consequence)
        if let Some(consequence) = node.child_by_field_name("consequence") {
            let cons_line = consequence.start_position().row as u32 + 1;
            let true_block = self.cfg.add_node(NodeKind::Block, (cons_line, cons_line));
            self.cfg.add_edge(if_node, true_block, EdgeKind::TrueBranch, Some(cond.clone()));
            
            self.current_node = true_block;
            self.visit_node(&consequence);
            
            if !self.is_exit_node(self.current_node) {
                self.cfg.add_edge(self.current_node, merge_node, EdgeKind::Fallthrough, None);
            }
        }

        // False branch (alternative)
        if let Some(alternative) = node.child_by_field_name("alternative") {
            let alt_line = alternative.start_position().row as u32 + 1;
            let false_block = self.cfg.add_node(NodeKind::Block, (alt_line, alt_line));
            self.cfg.add_edge(if_node, false_block, EdgeKind::FalseBranch, Some(cond));
            
            self.current_node = false_block;
            self.visit_node(&alternative);
            
            if !self.is_exit_node(self.current_node) {
                self.cfg.add_edge(self.current_node, merge_node, EdgeKind::Fallthrough, None);
            }
        } else {
            self.cfg.add_edge(if_node, merge_node, EdgeKind::FalseBranch, Some(cond));
        }

        self.current_node = merge_node;
    }

    fn visit_switch(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        
        let _scrutinee = node.child_by_field_name("value")
            .and_then(|c| c.utf8_text(self.source).ok())
            .unwrap_or("")
            .to_string();

        let switch_node = self.cfg.add_node(NodeKind::Match, (line, line));
        self.cfg.add_edge(self.current_node, switch_node, EdgeKind::Fallthrough, None);

        let end_line = node.end_position().row as u32 + 1;
        let merge_node = self.cfg.add_node(NodeKind::Block, (end_line, end_line));

        // Find switch_body
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for case in body.children(&mut cursor) {
                if case.kind() == "switch_case" || case.kind() == "switch_default" {
                    let case_line = case.start_position().row as u32 + 1;
                    let case_label = if case.kind() == "switch_default" {
                        "default".to_string()
                    } else {
                        case.child_by_field_name("value")
                            .and_then(|v| v.utf8_text(self.source).ok())
                            .unwrap_or("case")
                            .to_string()
                    };

                    let case_block = self.cfg.add_node(NodeKind::Block, (case_line, case_line));
                    self.cfg.add_edge(switch_node, case_block, EdgeKind::MatchArm, Some(case_label));
                    
                    self.current_node = case_block;
                    
                    // Visit case body statements
                    let mut stmt_cursor = case.walk();
                    for stmt in case.children(&mut stmt_cursor) {
                        if stmt.is_named() && stmt.kind() != "identifier" && !stmt.kind().contains("comment") {
                            self.visit_node(&stmt);
                        }
                    }
                    
                    if !self.is_exit_node(self.current_node) {
                        self.cfg.add_edge(self.current_node, merge_node, EdgeKind::Fallthrough, None);
                    }
                }
            }
        }

        self.current_node = merge_node;
    }

    fn visit_for(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        
        let header = self.cfg.add_node(NodeKind::Loop, (line, line));
        self.cfg.add_edge(self.current_node, header, EdgeKind::Fallthrough, None);

        let body_line = line + 1;
        let body = self.cfg.add_node(NodeKind::Block, (body_line, body_line));
        self.cfg.add_edge(header, body, EdgeKind::TrueBranch, Some("iteration".into()));

        let end_line = node.end_position().row as u32 + 1;
        let exit = self.cfg.add_node(NodeKind::Block, (end_line, end_line));
        self.cfg.add_edge(header, exit, EdgeKind::LoopExit, None);

        self.loop_stack.push((header, exit));
        self.current_node = body;
        
        if let Some(body_node) = node.child_by_field_name("body") {
            self.visit_node(&body_node);
        }
        
        if !self.is_exit_node(self.current_node) {
            self.cfg.add_edge(self.current_node, header, EdgeKind::LoopBack, None);
        }
        
        self.loop_stack.pop();
        self.current_node = exit;
    }

    fn visit_while(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        
        let cond = node.child_by_field_name("condition")
            .and_then(|c| c.utf8_text(self.source).ok())
            .unwrap_or("")
            .to_string();
        
        let header = self.cfg.add_node(NodeKind::Loop, (line, line));
        self.cfg.add_edge(self.current_node, header, EdgeKind::Fallthrough, None);

        let body_line = line + 1;
        let body = self.cfg.add_node(NodeKind::Block, (body_line, body_line));
        self.cfg.add_edge(header, body, EdgeKind::TrueBranch, Some(cond.clone()));

        let end_line = node.end_position().row as u32 + 1;
        let exit = self.cfg.add_node(NodeKind::Block, (end_line, end_line));
        self.cfg.add_edge(header, exit, EdgeKind::LoopExit, Some(format!("!{}", cond)));

        self.loop_stack.push((header, exit));
        self.current_node = body;
        
        if let Some(body_node) = node.child_by_field_name("body") {
            self.visit_node(&body_node);
        }
        
        if !self.is_exit_node(self.current_node) {
            self.cfg.add_edge(self.current_node, header, EdgeKind::LoopBack, None);
        }
        
        self.loop_stack.pop();
        self.current_node = exit;
    }

    fn visit_do_while(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        
        let cond = node.child_by_field_name("condition")
            .and_then(|c| c.utf8_text(self.source).ok())
            .unwrap_or("")
            .to_string();
        
        let body = self.cfg.add_node(NodeKind::Block, (line, line));
        self.cfg.add_edge(self.current_node, body, EdgeKind::Fallthrough, None);

        let header = self.cfg.add_node(NodeKind::Loop, (line, line));
        let end_line = node.end_position().row as u32 + 1;
        let exit = self.cfg.add_node(NodeKind::Block, (end_line, end_line));

        self.loop_stack.push((header, exit));
        self.current_node = body;
        
        if let Some(body_node) = node.child_by_field_name("body") {
            self.visit_node(&body_node);
        }
        
        if !self.is_exit_node(self.current_node) {
            self.cfg.add_edge(self.current_node, header, EdgeKind::Fallthrough, None);
        }
        
        self.cfg.add_edge(header, body, EdgeKind::TrueBranch, Some(cond.clone()));
        self.cfg.add_edge(header, exit, EdgeKind::LoopExit, Some(format!("!{}", cond)));
        
        self.loop_stack.pop();
        self.current_node = exit;
    }

    fn visit_return(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        handle_return(self.cfg, &mut self.current_node, line);
    }

    fn visit_break(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        handle_break(self.cfg, &mut self.current_node, &self.loop_stack, line);
    }

    fn visit_continue(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        handle_continue(self.cfg, &mut self.current_node, &self.loop_stack, line);
    }

    fn visit_throw(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        handle_panic(self.cfg, &mut self.current_node, line);
    }

    fn visit_try(&mut self, node: &tree_sitter::Node) {
        let end_line = node.end_position().row as u32 + 1;
        let merge_node = self.cfg.add_node(NodeKind::Block, (end_line, end_line));

        // Try body
        if let Some(body) = node.child_by_field_name("body") {
            self.visit_node(&body);
        }
        
        let after_try = self.current_node;
        
        // Catch handler
        if let Some(handler) = node.child_by_field_name("handler") {
            let catch_line = handler.start_position().row as u32 + 1;
            let catch_block = self.cfg.add_node(NodeKind::Block, (catch_line, catch_line));
            // Exception edge from try block
            self.cfg.add_edge(after_try, catch_block, EdgeKind::Exception, Some("catch".into()));
            
            self.current_node = catch_block;
            if let Some(body) = handler.child_by_field_name("body") {
                self.visit_node(&body);
            }
            
            if !self.is_exit_node(self.current_node) {
                self.cfg.add_edge(self.current_node, merge_node, EdgeKind::Fallthrough, None);
            }
        }
        
        // Connect try to merge
        if !self.is_exit_node(after_try) {
            self.cfg.add_edge(after_try, merge_node, EdgeKind::Fallthrough, None);
        }

        // Finally handler
        if let Some(finalizer) = node.child_by_field_name("finalizer") {
            let finally_line = finalizer.start_position().row as u32 + 1;
            let finally_block = self.cfg.add_node(NodeKind::Block, (finally_line, finally_line));
            self.cfg.add_edge(merge_node, finally_block, EdgeKind::Fallthrough, None);
            
            self.current_node = finally_block;
            self.visit_node(&finalizer);
            
            let final_merge = self.cfg.add_node(NodeKind::Block, (end_line, end_line));
            if !self.is_exit_node(self.current_node) {
                self.cfg.add_edge(self.current_node, final_merge, EdgeKind::Fallthrough, None);
            }
            self.current_node = final_merge;
        } else {
            self.current_node = merge_node;
        }
    }

    fn visit_expression_statement(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        
        // Check for process.exit or similar
        if let Ok(text) = node.utf8_text(self.source) {
            if is_ts_exit_call(text) {
                let exit_node = self.cfg.add_node(NodeKind::Exit, (line, line));
                self.cfg.add_edge(self.current_node, exit_node, EdgeKind::Fallthrough, None);
                self.current_node = exit_node;
                return;
            }
        }

        let stmt = Statement {
            line,
            kind: StatementKind::Expression,
            identifiers: Vec::new(),
            text: node.utf8_text(self.source).unwrap_or("expr").to_string(),
        };
        self.cfg.add_statement(self.current_node, stmt);
        self.update_source_range(line);
    }

    fn visit_declaration(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        
        let stmt = Statement {
            line,
            kind: StatementKind::Declaration,
            identifiers: Vec::new(),
            text: "declaration".into(),
        };
        self.cfg.add_statement(self.current_node, stmt);
        self.update_source_range(line);
    }

    fn is_exit_node(&self, node_id: NodeId) -> bool {
        if let Some(node) = self.cfg.nodes.get(node_id as usize) {
            matches!(
                node.kind,
                NodeKind::Return | NodeKind::Exit | NodeKind::Panic | NodeKind::Break | NodeKind::Continue
            )
        } else {
            false
        }
    }

    fn update_source_range(&mut self, line: u32) {
        if let Some(node) = self.cfg.nodes.get_mut(self.current_node as usize) {
            if node.source_range.0 == 0 {
                node.source_range.0 = line;
            }
            node.source_range.1 = line;
        }
    }
}

fn is_ts_exit_call(text: &str) -> bool {
    text.contains("process.exit")
        || text.contains("Deno.exit")
        || text.contains("sys.exit")
}

// ============================================================================
// Python CFG Builder (using tree-sitter)
// ============================================================================

/// Build a CFG from a Python function using tree-sitter
pub fn build_cfg_python(
    node: tree_sitter::Node,
    source: &[u8],
    path: &Path,
) -> Option<ControlFlowGraph> {
    let kind = node.kind();
    if !matches!(kind, "function_definition") {
        return None;
    }

    let name = node.child_by_field_name("name")
        .and_then(|n| n.utf8_text(source).ok())
        .unwrap_or("anonymous")
        .to_string();
    
    let start_line = node.start_position().row as u32 + 1;

    let mut cfg = ControlFlowGraph::new(name, path.to_path_buf(), start_line, CfgLanguage::Python);
    
    let body = node.child_by_field_name("body")?;
    
    let mut builder = PyCfgBuilder::new(&mut cfg, source);
    builder.visit_node(&body);
    builder.finalize();

    cfg.mark_exits();
    cfg.compute_reachability();
    Some(cfg)
}

struct PyCfgBuilder<'a> {
    cfg: &'a mut ControlFlowGraph,
    source: &'a [u8],
    current_node: NodeId,
    loop_stack: Vec<(NodeId, NodeId)>,
}

impl<'a> PyCfgBuilder<'a> {
    fn new(cfg: &'a mut ControlFlowGraph, source: &'a [u8]) -> Self {
        let entry = cfg.add_node(NodeKind::Entry, (0, 0));
        cfg.entry = entry;

        let block = cfg.add_node(NodeKind::Block, (0, 0));
        cfg.add_edge(entry, block, EdgeKind::Fallthrough, None);

        Self {
            cfg,
            source,
            current_node: block,
            loop_stack: Vec::new(),
        }
    }

    fn finalize(&mut self) {
        add_implicit_return_if_needed(self.cfg, self.current_node);
    }

    fn visit_node(&mut self, node: &tree_sitter::Node) {
        match node.kind() {
            "block" => self.visit_block(node),
            "if_statement" => self.visit_if(node),
            "match_statement" => self.visit_match(node),
            "for_statement" => self.visit_for(node),
            "while_statement" => self.visit_while(node),
            "return_statement" => self.visit_return(node),
            "break_statement" => self.visit_break(node),
            "continue_statement" => self.visit_continue(node),
            "raise_statement" => self.visit_raise(node),
            "try_statement" => self.visit_try(node),
            "expression_statement" => self.visit_expression(node),
            "assignment" => self.visit_assignment(node),
            _ => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    self.visit_node(&child);
                }
            }
        }
    }

    fn visit_block(&mut self, node: &tree_sitter::Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.is_named() {
                self.visit_node(&child);
            }
        }
    }

    fn visit_if(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        
        let cond = node.child_by_field_name("condition")
            .and_then(|c| c.utf8_text(self.source).ok())
            .unwrap_or("")
            .to_string();

        let if_node = self.cfg.add_node(NodeKind::If, (line, line));
        self.cfg.add_edge(self.current_node, if_node, EdgeKind::Fallthrough, None);

        let end_line = node.end_position().row as u32 + 1;
        let merge_node = self.cfg.add_node(NodeKind::Block, (end_line, end_line));

        // True branch (consequence)
        if let Some(consequence) = node.child_by_field_name("consequence") {
            let cons_line = consequence.start_position().row as u32 + 1;
            let true_block = self.cfg.add_node(NodeKind::Block, (cons_line, cons_line));
            self.cfg.add_edge(if_node, true_block, EdgeKind::TrueBranch, Some(cond.clone()));
            
            self.current_node = true_block;
            self.visit_node(&consequence);
            
            if !self.is_exit_node(self.current_node) {
                self.cfg.add_edge(self.current_node, merge_node, EdgeKind::Fallthrough, None);
            }
        }

        // Elif/else branches (alternative)
        if let Some(alternative) = node.child_by_field_name("alternative") {
            let alt_line = alternative.start_position().row as u32 + 1;
            let false_block = self.cfg.add_node(NodeKind::Block, (alt_line, alt_line));
            self.cfg.add_edge(if_node, false_block, EdgeKind::FalseBranch, Some(cond));
            
            self.current_node = false_block;
            self.visit_node(&alternative);
            
            if !self.is_exit_node(self.current_node) {
                self.cfg.add_edge(self.current_node, merge_node, EdgeKind::Fallthrough, None);
            }
        } else {
            self.cfg.add_edge(if_node, merge_node, EdgeKind::FalseBranch, Some(cond));
        }

        self.current_node = merge_node;
    }

    fn visit_match(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        
        let match_node = self.cfg.add_node(NodeKind::Match, (line, line));
        self.cfg.add_edge(self.current_node, match_node, EdgeKind::Fallthrough, None);

        let end_line = node.end_position().row as u32 + 1;
        let merge_node = self.cfg.add_node(NodeKind::Block, (end_line, end_line));

        // Process case clauses
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "case_clause" {
                let case_line = child.start_position().row as u32 + 1;
                let pattern = child.child_by_field_name("pattern")
                    .and_then(|p| p.utf8_text(self.source).ok())
                    .unwrap_or("_")
                    .to_string();

                let case_block = self.cfg.add_node(NodeKind::Block, (case_line, case_line));
                self.cfg.add_edge(match_node, case_block, EdgeKind::MatchArm, Some(pattern));
                
                self.current_node = case_block;
                
                if let Some(body) = child.child_by_field_name("consequence") {
                    self.visit_node(&body);
                }
                
                if !self.is_exit_node(self.current_node) {
                    self.cfg.add_edge(self.current_node, merge_node, EdgeKind::Fallthrough, None);
                }
            }
        }

        self.current_node = merge_node;
    }

    fn visit_for(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        
        let header = self.cfg.add_node(NodeKind::Loop, (line, line));
        self.cfg.add_edge(self.current_node, header, EdgeKind::Fallthrough, None);

        let body_node = self.cfg.add_node(NodeKind::Block, (line + 1, line + 1));
        self.cfg.add_edge(header, body_node, EdgeKind::TrueBranch, Some("iteration".into()));

        let end_line = node.end_position().row as u32 + 1;
        let exit = self.cfg.add_node(NodeKind::Block, (end_line, end_line));
        self.cfg.add_edge(header, exit, EdgeKind::LoopExit, None);

        self.loop_stack.push((header, exit));
        self.current_node = body_node;
        
        if let Some(body) = node.child_by_field_name("body") {
            self.visit_node(&body);
        }
        
        if !self.is_exit_node(self.current_node) {
            self.cfg.add_edge(self.current_node, header, EdgeKind::LoopBack, None);
        }
        
        self.loop_stack.pop();
        self.current_node = exit;
    }

    fn visit_while(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        
        let cond = node.child_by_field_name("condition")
            .and_then(|c| c.utf8_text(self.source).ok())
            .unwrap_or("")
            .to_string();
        
        let header = self.cfg.add_node(NodeKind::Loop, (line, line));
        self.cfg.add_edge(self.current_node, header, EdgeKind::Fallthrough, None);

        let body_node = self.cfg.add_node(NodeKind::Block, (line + 1, line + 1));
        self.cfg.add_edge(header, body_node, EdgeKind::TrueBranch, Some(cond.clone()));

        let end_line = node.end_position().row as u32 + 1;
        let exit = self.cfg.add_node(NodeKind::Block, (end_line, end_line));
        self.cfg.add_edge(header, exit, EdgeKind::LoopExit, Some(format!("not {}", cond)));

        self.loop_stack.push((header, exit));
        self.current_node = body_node;
        
        if let Some(body) = node.child_by_field_name("body") {
            self.visit_node(&body);
        }
        
        if !self.is_exit_node(self.current_node) {
            self.cfg.add_edge(self.current_node, header, EdgeKind::LoopBack, None);
        }
        
        self.loop_stack.pop();
        self.current_node = exit;
    }

    fn visit_return(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        handle_return(self.cfg, &mut self.current_node, line);
    }

    fn visit_break(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        handle_break(self.cfg, &mut self.current_node, &self.loop_stack, line);
    }

    fn visit_continue(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        handle_continue(self.cfg, &mut self.current_node, &self.loop_stack, line);
    }

    fn visit_raise(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        handle_panic(self.cfg, &mut self.current_node, line);
    }

    fn visit_try(&mut self, node: &tree_sitter::Node) {
        let end_line = node.end_position().row as u32 + 1;
        let merge_node = self.cfg.add_node(NodeKind::Block, (end_line, end_line));

        // Try body
        if let Some(body) = node.child_by_field_name("body") {
            self.visit_node(&body);
        }
        
        let after_try = self.current_node;
        
        // Except handlers
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "except_clause" {
                let except_line = child.start_position().row as u32 + 1;
                let except_block = self.cfg.add_node(NodeKind::Block, (except_line, except_line));
                self.cfg.add_edge(after_try, except_block, EdgeKind::Exception, Some("except".into()));
                
                self.current_node = except_block;
                let mut handler_cursor = child.walk();
                for handler_child in child.children(&mut handler_cursor) {
                    if handler_child.is_named() {
                        self.visit_node(&handler_child);
                    }
                }
                
                if !self.is_exit_node(self.current_node) {
                    self.cfg.add_edge(self.current_node, merge_node, EdgeKind::Fallthrough, None);
                }
            }
        }
        
        if !self.is_exit_node(after_try) {
            self.cfg.add_edge(after_try, merge_node, EdgeKind::Fallthrough, None);
        }

        self.current_node = merge_node;
    }

    fn visit_expression(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        
        if let Ok(text) = node.utf8_text(self.source) {
            if text.contains("sys.exit") || text.contains("exit(") || text.contains("os._exit") {
                let exit_node = self.cfg.add_node(NodeKind::Exit, (line, line));
                self.cfg.add_edge(self.current_node, exit_node, EdgeKind::Fallthrough, None);
                self.current_node = exit_node;
                return;
            }
        }

        let stmt = Statement {
            line,
            kind: StatementKind::Expression,
            identifiers: Vec::new(),
            text: "expression".into(),
        };
        self.cfg.add_statement(self.current_node, stmt);
        self.update_source_range(line);
    }

    fn visit_assignment(&mut self, node: &tree_sitter::Node) {
        let line = node.start_position().row as u32 + 1;
        let stmt = Statement {
            line,
            kind: StatementKind::Assignment,
            identifiers: Vec::new(),
            text: "assignment".into(),
        };
        self.cfg.add_statement(self.current_node, stmt);
        self.update_source_range(line);
    }

    fn is_exit_node(&self, node_id: NodeId) -> bool {
        if let Some(node) = self.cfg.nodes.get(node_id as usize) {
            matches!(
                node.kind,
                NodeKind::Return | NodeKind::Exit | NodeKind::Panic | NodeKind::Break | NodeKind::Continue
            )
        } else {
            false
        }
    }

    fn update_source_range(&mut self, line: u32) {
        if let Some(node) = self.cfg.nodes.get_mut(self.current_node as usize) {
            if node.source_range.0 == 0 {
                node.source_range.0 = line;
            }
            node.source_range.1 = line;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cfg_basic() {
        let mut cfg = ControlFlowGraph::new(
            "test_fn".to_string(),
            PathBuf::from("test.rs"),
            1,
            CfgLanguage::Rust,
        );

        let entry = cfg.add_node(NodeKind::Entry, (1, 1));
        cfg.entry = entry;

        let block1 = cfg.add_node(NodeKind::Block, (2, 3));
        let ret = cfg.add_node(NodeKind::Return, (4, 4));

        cfg.add_edge(entry, block1, EdgeKind::Fallthrough, None);
        cfg.add_edge(block1, ret, EdgeKind::Fallthrough, None);

        cfg.mark_exits();
        cfg.compute_reachability();

        assert_eq!(cfg.exits.len(), 1);
        assert!(cfg.nodes[entry as usize].reachable);
        assert!(cfg.nodes[block1 as usize].reachable);
        assert!(cfg.nodes[ret as usize].reachable);
    }

    #[test]
    fn test_cfg_unreachable() {
        let mut cfg = ControlFlowGraph::new(
            "test_fn".to_string(),
            PathBuf::from("test.rs"),
            1,
            CfgLanguage::Rust,
        );

        let entry = cfg.add_node(NodeKind::Entry, (1, 1));
        cfg.entry = entry;

        let ret = cfg.add_node(NodeKind::Return, (2, 2));
        let dead = cfg.add_node(NodeKind::Block, (3, 3)); // No edge to this node

        cfg.add_edge(entry, ret, EdgeKind::Fallthrough, None);

        cfg.mark_exits();
        cfg.compute_reachability();

        assert!(!cfg.nodes[dead as usize].reachable);
        assert_eq!(cfg.unreachable_nodes().len(), 1);
    }

    #[test]
    fn test_cfg_exit_paths() {
        let mut cfg = ControlFlowGraph::new(
            "test_fn".to_string(),
            PathBuf::from("test.rs"),
            1,
            CfgLanguage::Rust,
        );

        let entry = cfg.add_node(NodeKind::Entry, (1, 1));
        cfg.entry = entry;

        let if_node = cfg.add_node(NodeKind::If, (2, 2));
        let ret1 = cfg.add_node(NodeKind::Return, (3, 3));
        let ret2 = cfg.add_node(NodeKind::Return, (5, 5));

        cfg.add_edge(entry, if_node, EdgeKind::Fallthrough, None);
        cfg.add_edge(if_node, ret1, EdgeKind::TrueBranch, Some("x > 0".into()));
        cfg.add_edge(if_node, ret2, EdgeKind::FalseBranch, Some("x > 0".into()));

        cfg.mark_exits();
        cfg.compute_reachability();

        let paths = cfg.all_exit_paths();
        assert_eq!(paths.len(), 2);
    }
}
