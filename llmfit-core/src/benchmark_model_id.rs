//! Normalise provider-reported model ids before storing or sharing benchmarks.
//!
//! llama-server echoes its `--model` argument (often an absolute GGUF path);
//! Ollama-backed runs may report a blob path under `.ollama/models/blobs/`.
//! Hub-style ids such as `meta-llama/Llama-3.1-8B-Instruct` must survive.

/// Outcome of [`normalize_benchmark_model_id`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedModelId {
    pub id: String,
    /// The stored id is a basename stripped from a machine-specific absolute
    /// path that does not identify the model for human readers (not a GGUF
    /// file name or Ollama `sha256-…` blob id).
    pub opaque: bool,
}

/// Returns true when `id` looks like an absolute filesystem path.
fn is_absolute_path(id: &str) -> bool {
    id.starts_with('/')
        || id.starts_with('\\')
        || matches!(id.as_bytes(), [_, b':', b'/' | b'\\', ..])
}

fn has_path_separator(id: &str) -> bool {
    id.contains(std::path::MAIN_SEPARATOR)
        || (std::path::MAIN_SEPARATOR != '/' && id.contains('/'))
        || (std::path::MAIN_SEPARATOR != '\\' && id.contains('\\'))
}

fn basename(id: &str) -> &str {
    id.rsplit(['/', '\\']).next().unwrap_or(id)
}

/// Ollama content-addressed blob file name (`sha256-` + 64 hex digits).
fn is_ollama_blob_basename(name: &str) -> bool {
    match name.strip_prefix("sha256-") {
        Some(rest) => rest.len() == 64 && rest.bytes().all(|b| b.is_ascii_hexdigit()),
        None => false,
    }
}

/// Apply storage/share policy to a raw model id.
pub fn normalize_benchmark_model_id(id: &str) -> NormalizedModelId {
    // (1) Absolute path to a `.gguf` file → bare file name (#819).
    if is_absolute_path(id) && id.to_ascii_lowercase().ends_with(".gguf") {
        return NormalizedModelId {
            id: basename(id).to_string(),
            opaque: false,
        };
    }

    // (2) Path whose final component is an Ollama blob id.
    if has_path_separator(id) {
        let base = basename(id);
        if is_ollama_blob_basename(base) {
            return NormalizedModelId {
                id: base.to_string(),
                opaque: false,
            };
        }
    }

    // (3) Other absolute paths → basename only; flag as opaque for local audit.
    if is_absolute_path(id) {
        return NormalizedModelId {
            id: basename(id).to_string(),
            opaque: true,
        };
    }

    NormalizedModelId {
        id: id.to_string(),
        opaque: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_absolute_gguf_paths() {
        let n = normalize_benchmark_model_id("/home/user/gguf/SmolLM2-135M-Instruct-Q4_K_M.gguf");
        assert_eq!(n.id, "SmolLM2-135M-Instruct-Q4_K_M.gguf");
        assert!(!n.opaque);
        let n = normalize_benchmark_model_id(r"C:\models\phi-4-Q4_K_M.GGUF");
        assert_eq!(n.id, "phi-4-Q4_K_M.GGUF");
        assert!(!n.opaque);
    }

    #[test]
    fn preserves_bare_gguf_filename() {
        let n = normalize_benchmark_model_id("model.gguf");
        assert_eq!(n.id, "model.gguf");
        assert!(!n.opaque);
    }

    #[test]
    fn strips_ollama_blob_paths() {
        let path = "/usr/share/ollama/.ollama/models/blobs/sha256-dde5aa3fc5ffc17176b5e8bdc82f587b24b2678c6c66101bf7da77af9f7ccdff";
        let n = normalize_benchmark_model_id(path);
        assert_eq!(
            n.id,
            "sha256-dde5aa3fc5ffc17176b5e8bdc82f587b24b2678c6c66101bf7da77af9f7ccdff"
        );
        assert!(!n.opaque);
    }

    #[test]
    fn preserves_logical_hub_ids() {
        assert_eq!(
            normalize_benchmark_model_id("meta-llama/Llama-3.1-8B-Instruct").id,
            "meta-llama/Llama-3.1-8B-Instruct"
        );
        assert_eq!(
            normalize_benchmark_model_id("unsloth/Qwen3-4B-GGUF").id,
            "unsloth/Qwen3-4B-GGUF"
        );
        assert_eq!(
            normalize_benchmark_model_id("llama3.1:8b").id,
            "llama3.1:8b"
        );
    }

    #[test]
    fn preserves_relative_gguf_paths() {
        let n = normalize_benchmark_model_id("models/foo.gguf");
        assert_eq!(n.id, "models/foo.gguf");
        assert!(!n.opaque);
    }

    #[test]
    fn preserves_hub_style_gguf_reference() {
        let id = "hf.co/bartowski/SmolLM2-135M-Instruct-GGUF/SmolLM2-135M-Instruct-Q4_K_M.gguf";
        let n = normalize_benchmark_model_id(id);
        assert_eq!(n.id, id);
        assert!(!n.opaque);
    }

    #[test]
    fn opaque_absolute_non_gguf_paths() {
        let n = normalize_benchmark_model_id("/models/config.json");
        assert_eq!(n.id, "config.json");
        assert!(n.opaque);
    }
}
