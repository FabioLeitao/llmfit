//! Controlled prompt sizing for benchmark runs (no tokenizer; server counts win).

use std::fs;
use std::path::Path;

/// Prompt body and optional token target for reporting (`requested` vs measured).
#[derive(Debug, Clone)]
pub struct BenchPromptSpec {
    pub text: String,
    pub requested_tokens: Option<u32>,
}

const SIZING_BLOCK: &str = include_str!("../data/bench_sizing_en.txt");

/// Conservative chars-per-token guess when sizing without a tokenizer.
const ESTIMATED_CHARS_PER_TOKEN: usize = 4;

/// Maximum `--prompt-tokens` (≈512 KiB of prompt text at 4 chars/token).
pub const MAX_PROMPT_TOKENS: u32 = 131_072;

/// Minimum `--prompt-tokens`.
pub const MIN_PROMPT_TOKENS: u32 = 1;

/// Maximum size for `--prompt-file` (8 MiB).
pub const MAX_PROMPT_FILE_BYTES: u64 = 8 * 1024 * 1024;

pub fn validate_prompt_tokens(target: u32) -> Result<(), String> {
    if target < MIN_PROMPT_TOKENS {
        return Err(format!(
            "--prompt-tokens must be at least {MIN_PROMPT_TOKENS}, got {target}"
        ));
    }
    if target > MAX_PROMPT_TOKENS {
        return Err(format!(
            "--prompt-tokens must be at most {MAX_PROMPT_TOKENS}, got {target}"
        ));
    }
    Ok(())
}

/// Prefix that makes each sized run unique so servers do not reuse prefix KV cache.
pub fn prompt_with_run_nonce(base: &str, run_index: usize) -> String {
    format!("Run-ID: {:04}\n{}", run_index + 1, base)
}

/// Build prompt text approximating `target_tokens` by repeating the sizing block.
pub fn prompt_text_for_target_tokens(target_tokens: u32) -> String {
    let target_chars = target_tokens as usize * ESTIMATED_CHARS_PER_TOKEN;
    if target_chars == 0 {
        return String::new();
    }
    let block = SIZING_BLOCK.trim();
    if block.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(target_chars + block.len());
    while out.len() < target_chars {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(block);
    }
    out
}

pub fn read_prompt_file(path: &Path) -> Result<String, String> {
    let meta = fs::metadata(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!(
            "prompt file {} is not a regular file",
            path.display()
        ));
    }
    let len = meta.len();
    if len > MAX_PROMPT_FILE_BYTES {
        return Err(format!(
            "prompt file {} is {} bytes (limit {} bytes)",
            path.display(),
            len,
            MAX_PROMPT_FILE_BYTES
        ));
    }
    fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))
}

/// Resolve CLI `--prompt-file` / `--prompt-tokens` into a single spec.
pub fn resolve_bench_prompt(
    prompt_file: Option<&Path>,
    prompt_tokens: Option<u32>,
) -> Result<Option<BenchPromptSpec>, String> {
    match (prompt_file, prompt_tokens) {
        (None, None) => Ok(None),
        (Some(path), tokens) => {
            if let Some(target) = tokens {
                validate_prompt_tokens(target)?;
            }
            let text = read_prompt_file(path)?;
            if text.trim().is_empty() {
                return Err(format!("prompt file {} is empty", path.display()));
            }
            Ok(Some(BenchPromptSpec {
                text,
                requested_tokens: tokens,
            }))
        }
        (None, Some(target)) => {
            validate_prompt_tokens(target)?;
            Ok(Some(BenchPromptSpec {
                text: prompt_text_for_target_tokens(target),
                requested_tokens: Some(target),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_prompt_tokens_rejects_zero_and_huge() {
        assert!(validate_prompt_tokens(0).is_err());
        assert!(validate_prompt_tokens(MAX_PROMPT_TOKENS + 1).is_err());
        assert!(validate_prompt_tokens(1500).is_ok());
    }

    #[test]
    fn resolve_rejects_empty_file() {
        let dir = std::env::temp_dir().join(format!("llmfit-prompt-{}", std::process::id()));
        let path = dir.join("empty.txt");
        fs::create_dir_all(&dir).expect("temp dir");
        fs::write(&path, "  \n").expect("empty file");
        let err = resolve_bench_prompt(Some(&path), None).unwrap_err();
        assert!(err.contains("empty"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_file_with_token_target() {
        let dir = std::env::temp_dir().join(format!("llmfit-prompt2-{}", std::process::id()));
        let path = dir.join("body.txt");
        fs::create_dir_all(&dir).expect("temp dir");
        fs::write(&path, "hello").expect("file");
        let spec = resolve_bench_prompt(Some(&path), Some(100))
            .expect("resolve")
            .expect("some");
        assert_eq!(spec.text, "hello");
        assert_eq!(spec.requested_tokens, Some(100));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sizing_block_grows_with_target() {
        let small = prompt_text_for_target_tokens(50);
        let large = prompt_text_for_target_tokens(500);
        assert!(large.len() > small.len());
        assert!(large.contains(SIZING_BLOCK.trim()));
    }
}
