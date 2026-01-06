//! Coverage file parsing for dynamic tracing integration.
//!
//! This module parses common coverage file formats to determine
//! which lines/functions are covered by tests. This information
//! can be used to gate findings - code covered by tests is less
//! likely to be dead or unused.
//!
//! Supported formats:
//! - LCOV (lcov.info)
//! - Istanbul/NYC (coverage-final.json)
//! - Cobertura (coverage.xml)

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Coverage format identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CoverageFormat {
    /// LCOV format (lcov.info)
    Lcov,
    /// Istanbul/NYC JSON format (coverage-final.json)
    Istanbul,
    /// Cobertura XML format (coverage.xml)
    Cobertura,
}

impl CoverageFormat {
    /// Detect format from file path
    pub fn detect(path: &Path) -> Option<Self> {
        let filename = path.file_name()?.to_str()?;
        let extension = path.extension().and_then(|e| e.to_str());

        if filename.contains("lcov") || filename == "coverage.info" {
            Some(Self::Lcov)
        } else if extension == Some("json") && filename.contains("coverage") {
            Some(Self::Istanbul)
        } else if extension == Some("xml") && filename.contains("coverage") {
            Some(Self::Cobertura)
        } else {
            None
        }
    }
}

/// Coverage data for a single file
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileCoverage {
    /// Lines that were executed at least once
    pub lines_hit: HashSet<u32>,
    /// All lines that could be executed (instrumented)
    pub lines_found: HashSet<u32>,
    /// Functions that were executed at least once
    pub functions_hit: HashSet<String>,
    /// All functions that could be executed
    pub functions_found: HashSet<String>,
    /// Branches covered
    pub branches_hit: u32,
    /// Total branches
    pub branches_found: u32,
}

impl FileCoverage {
    /// Calculate line coverage percentage
    pub fn line_coverage_pct(&self) -> f32 {
        if self.lines_found.is_empty() {
            0.0
        } else {
            (self.lines_hit.len() as f32 / self.lines_found.len() as f32) * 100.0
        }
    }

    /// Calculate function coverage percentage
    pub fn function_coverage_pct(&self) -> f32 {
        if self.functions_found.is_empty() {
            0.0
        } else {
            (self.functions_hit.len() as f32 / self.functions_found.len() as f32) * 100.0
        }
    }

    /// Check if a specific line is covered
    pub fn is_line_covered(&self, line: u32) -> bool {
        self.lines_hit.contains(&line)
    }

    /// Check if a specific function is covered
    pub fn is_function_covered(&self, name: &str) -> bool {
        self.functions_hit.contains(name)
    }

    /// Check if a line is instrumented (could be covered)
    pub fn is_line_instrumented(&self, line: u32) -> bool {
        self.lines_found.contains(&line)
    }
}

/// Project-wide coverage data
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CoverageData {
    /// Coverage data per file
    pub files: HashMap<PathBuf, FileCoverage>,
    /// Source format
    pub format: Option<CoverageFormat>,
}

impl CoverageData {
    /// Create empty coverage data
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse coverage from a file, auto-detecting format
    pub fn from_file(path: &Path) -> Result<Self> {
        let format = CoverageFormat::detect(path)
            .with_context(|| format!("Could not detect coverage format for {}", path.display()))?;

        match format {
            CoverageFormat::Lcov => Self::parse_lcov(path),
            CoverageFormat::Istanbul => Self::parse_istanbul(path),
            CoverageFormat::Cobertura => Self::parse_cobertura(path),
        }
    }

    /// Parse LCOV format coverage
    pub fn parse_lcov(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let mut data = Self::new();
        data.format = Some(CoverageFormat::Lcov);

        let mut current_file: Option<PathBuf> = None;
        let mut current_coverage = FileCoverage::default();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            if let Some(path_str) = line.strip_prefix("SF:") {
                // Start of new file
                if let Some(prev_file) = current_file.take() {
                    data.files.insert(prev_file, std::mem::take(&mut current_coverage));
                }
                current_file = Some(PathBuf::from(path_str));
            } else if line.starts_with("DA:") {
                // Line data: DA:line_number,execution_count
                if let Some(rest) = line.strip_prefix("DA:") {
                    let parts: Vec<&str> = rest.split(',').collect();
                    if parts.len() >= 2 {
                        if let Ok(line_num) = parts[0].parse::<u32>() {
                            current_coverage.lines_found.insert(line_num);
                            if let Ok(count) = parts[1].parse::<u32>() {
                                if count > 0 {
                                    current_coverage.lines_hit.insert(line_num);
                                }
                            }
                        }
                    }
                }
            } else if line.starts_with("FN:") {
                // Function definition: FN:line_number,function_name
                if let Some(rest) = line.strip_prefix("FN:") {
                    let parts: Vec<&str> = rest.splitn(2, ',').collect();
                    if parts.len() >= 2 {
                        current_coverage.functions_found.insert(parts[1].to_string());
                    }
                }
            } else if line.starts_with("FNDA:") {
                // Function data: FNDA:execution_count,function_name
                if let Some(rest) = line.strip_prefix("FNDA:") {
                    let parts: Vec<&str> = rest.splitn(2, ',').collect();
                    if parts.len() >= 2 {
                        if let Ok(count) = parts[0].parse::<u32>() {
                            if count > 0 {
                                current_coverage.functions_hit.insert(parts[1].to_string());
                            }
                        }
                    }
                }
            } else if line.starts_with("BRDA:") {
                // Branch data: BRDA:line,block,branch,taken
                if let Some(rest) = line.strip_prefix("BRDA:") {
                    let parts: Vec<&str> = rest.split(',').collect();
                    if parts.len() >= 4 {
                        current_coverage.branches_found += 1;
                        if parts[3] != "-" && parts[3] != "0" {
                            current_coverage.branches_hit += 1;
                        }
                    }
                }
            } else if line == "end_of_record" {
                // End of current file record
                if let Some(prev_file) = current_file.take() {
                    data.files.insert(prev_file, std::mem::take(&mut current_coverage));
                }
            }
        }

        // Handle last file if no end_of_record
        if let Some(prev_file) = current_file {
            data.files.insert(prev_file, current_coverage);
        }

        Ok(data)
    }

    /// Parse Istanbul/NYC JSON format coverage
    pub fn parse_istanbul(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let mut data = Self::new();
        data.format = Some(CoverageFormat::Istanbul);

        // Istanbul format is a JSON object where keys are file paths
        // and values contain statement/branch/function coverage
        let parsed: serde_json::Value = serde_json::from_str(&content)
            .with_context(|| "Failed to parse Istanbul JSON")?;

        if let Some(obj) = parsed.as_object() {
            for (file_path, file_data) in obj {
                let mut file_cov = FileCoverage::default();

                // Parse statement map (s)
                if let Some(s) = file_data.get("s").and_then(|v| v.as_object()) {
                    for (_, count) in s {
                        if let Some(n) = count.as_u64() {
                            // We don't have line numbers here, so approximate
                            if n > 0 {
                                // Mark as covered (we'll need statementMap for lines)
                            }
                        }
                    }
                }

                // Parse function map (f)
                if let Some(f) = file_data.get("f").and_then(|v| v.as_object()) {
                    if let Some(fn_map) = file_data.get("fnMap").and_then(|v| v.as_object()) {
                        for (idx, count) in f {
                            if let Some(fn_info) = fn_map.get(idx) {
                                let name = fn_info
                                    .get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("anonymous")
                                    .to_string();
                                file_cov.functions_found.insert(name.clone());
                                if count.as_u64().unwrap_or(0) > 0 {
                                    file_cov.functions_hit.insert(name);
                                }
                            }
                        }
                    }
                }

                // Parse statement map for line coverage
                if let Some(stmt_map) = file_data.get("statementMap").and_then(|v| v.as_object()) {
                    if let Some(s) = file_data.get("s").and_then(|v| v.as_object()) {
                        for (idx, _stmt_info) in stmt_map {
                            if let Some(loc) = stmt_map
                                .get(idx)
                                .and_then(|v| v.get("start"))
                                .and_then(|v| v.get("line"))
                                .and_then(|v| v.as_u64())
                            {
                                let line = loc as u32;
                                file_cov.lines_found.insert(line);
                                if let Some(count) = s.get(idx).and_then(|v| v.as_u64()) {
                                    if count > 0 {
                                        file_cov.lines_hit.insert(line);
                                    }
                                }
                            }
                        }
                    }
                }

                // Parse branch coverage
                if let Some(b) = file_data.get("b").and_then(|v| v.as_object()) {
                    for (_, branches) in b {
                        if let Some(arr) = branches.as_array() {
                            for count in arr {
                                file_cov.branches_found += 1;
                                if count.as_u64().unwrap_or(0) > 0 {
                                    file_cov.branches_hit += 1;
                                }
                            }
                        }
                    }
                }

                data.files.insert(PathBuf::from(file_path), file_cov);
            }
        }

        Ok(data)
    }

    /// Parse Cobertura XML format coverage
    pub fn parse_cobertura(path: &Path) -> Result<Self> {
        // Simplified Cobertura parser - just read line coverage
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let mut data = Self::new();
        data.format = Some(CoverageFormat::Cobertura);

        // Very basic XML parsing for line coverage
        // In production, use a proper XML parser
        let mut current_file: Option<PathBuf> = None;
        let mut current_coverage = FileCoverage::default();

        for line in content.lines() {
            let line = line.trim();
            
            // Look for <class filename="...">
            if line.contains("<class") && line.contains("filename=") {
                if let Some(prev_file) = current_file.take() {
                    data.files.insert(prev_file, std::mem::take(&mut current_coverage));
                }
                
                // Extract filename
                if let Some(start) = line.find("filename=\"") {
                    let rest = &line[start + 10..];
                    if let Some(end) = rest.find('"') {
                        current_file = Some(PathBuf::from(&rest[..end]));
                    }
                }
            }
            
            // Look for <line number="X" hits="Y"/>
            if line.contains("<line") && line.contains("number=") && line.contains("hits=") {
                let mut line_num = None;
                let mut hits = None;
                
                if let Some(start) = line.find("number=\"") {
                    let rest = &line[start + 8..];
                    if let Some(end) = rest.find('"') {
                        line_num = rest[..end].parse::<u32>().ok();
                    }
                }
                
                if let Some(start) = line.find("hits=\"") {
                    let rest = &line[start + 6..];
                    if let Some(end) = rest.find('"') {
                        hits = rest[..end].parse::<u32>().ok();
                    }
                }
                
                if let (Some(ln), Some(h)) = (line_num, hits) {
                    current_coverage.lines_found.insert(ln);
                    if h > 0 {
                        current_coverage.lines_hit.insert(ln);
                    }
                }
            }
        }

        // Handle last file
        if let Some(prev_file) = current_file {
            data.files.insert(prev_file, current_coverage);
        }

        Ok(data)
    }

    /// Get coverage for a specific file
    pub fn get_file_coverage(&self, path: &Path) -> Option<&FileCoverage> {
        // Try exact match first
        if let Some(cov) = self.files.get(path) {
            return Some(cov);
        }

        // Try normalized path matching
        let normalized = path.to_string_lossy().replace('\\', "/");
        for (file_path, cov) in &self.files {
            let file_normalized = file_path.to_string_lossy().replace('\\', "/");
            if file_normalized.ends_with(&normalized) || normalized.ends_with(&file_normalized) {
                return Some(cov);
            }
        }

        None
    }

    /// Check if a specific line in a file is covered
    pub fn is_line_covered(&self, path: &Path, line: u32) -> Option<bool> {
        self.get_file_coverage(path).map(|c| c.is_line_covered(line))
    }

    /// Check if a specific function is covered
    pub fn is_function_covered(&self, path: &Path, name: &str) -> Option<bool> {
        self.get_file_coverage(path).map(|c| c.is_function_covered(name))
    }

    /// Get overall coverage statistics
    pub fn stats(&self) -> CoverageStats {
        let mut total_lines_hit = 0;
        let mut total_lines_found = 0;
        let mut total_functions_hit = 0;
        let mut total_functions_found = 0;
        let mut total_branches_hit = 0;
        let mut total_branches_found = 0;

        for cov in self.files.values() {
            total_lines_hit += cov.lines_hit.len();
            total_lines_found += cov.lines_found.len();
            total_functions_hit += cov.functions_hit.len();
            total_functions_found += cov.functions_found.len();
            total_branches_hit += cov.branches_hit as usize;
            total_branches_found += cov.branches_found as usize;
        }

        CoverageStats {
            files: self.files.len(),
            lines_hit: total_lines_hit,
            lines_found: total_lines_found,
            line_coverage_pct: if total_lines_found > 0 {
                (total_lines_hit as f32 / total_lines_found as f32) * 100.0
            } else {
                0.0
            },
            functions_hit: total_functions_hit,
            functions_found: total_functions_found,
            function_coverage_pct: if total_functions_found > 0 {
                (total_functions_hit as f32 / total_functions_found as f32) * 100.0
            } else {
                0.0
            },
            branches_hit: total_branches_hit,
            branches_found: total_branches_found,
            branch_coverage_pct: if total_branches_found > 0 {
                (total_branches_hit as f32 / total_branches_found as f32) * 100.0
            } else {
                0.0
            },
        }
    }
}

/// Overall coverage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageStats {
    pub files: usize,
    pub lines_hit: usize,
    pub lines_found: usize,
    pub line_coverage_pct: f32,
    pub functions_hit: usize,
    pub functions_found: usize,
    pub function_coverage_pct: f32,
    pub branches_hit: usize,
    pub branches_found: usize,
    pub branch_coverage_pct: f32,
}

/// Coverage information to attach to a finding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageInfo {
    /// Whether the code is covered by tests
    pub is_covered: bool,
    /// Line coverage percentage for the file
    pub line_coverage_pct: f32,
    /// Whether the specific function is covered (if applicable)
    pub function_covered: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lcov_parsing() {
        let lcov_content = r#"TN:
SF:/path/to/file.ts
FN:1,main
FNDA:10,main
DA:1,10
DA:2,10
DA:3,0
LH:2
LF:3
end_of_record
"#;
        
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_lcov.info");
        std::fs::write(&temp_file, lcov_content).unwrap();

        let coverage = CoverageData::parse_lcov(&temp_file).unwrap();
        
        let file_cov = coverage.files.get(&PathBuf::from("/path/to/file.ts")).unwrap();
        assert!(file_cov.is_line_covered(1));
        assert!(file_cov.is_line_covered(2));
        assert!(!file_cov.is_line_covered(3));
        assert!(file_cov.is_function_covered("main"));

        std::fs::remove_file(&temp_file).ok();
    }

    #[test]
    fn test_coverage_format_detection() {
        assert_eq!(
            CoverageFormat::detect(Path::new("coverage/lcov.info")),
            Some(CoverageFormat::Lcov)
        );
        assert_eq!(
            CoverageFormat::detect(Path::new("coverage-final.json")),
            Some(CoverageFormat::Istanbul)
        );
        assert_eq!(
            CoverageFormat::detect(Path::new("coverage.xml")),
            Some(CoverageFormat::Cobertura)
        );
    }

    #[test]
    fn test_file_coverage_percentages() {
        let mut cov = FileCoverage::default();
        cov.lines_found = [1, 2, 3, 4, 5].into_iter().collect();
        cov.lines_hit = [1, 2, 3].into_iter().collect();

        assert!((cov.line_coverage_pct() - 60.0).abs() < 0.1);
    }
}
