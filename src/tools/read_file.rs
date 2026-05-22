use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadFileInput {
    /// Absolute path to the file to read
    pub file_path: String,
}

#[derive(Debug, Serialize)]
pub struct ReadFileResult {
    pub content: String,
}

pub struct ReadFileTool;

impl ReadFileTool {
    pub fn read(&self, input: ReadFileInput) -> Result<ReadFileResult> {
        let content = std::fs::read_to_string(&input.file_path)
            .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {}", input.file_path, e))?;
        Ok(ReadFileResult { content })
    }
}
