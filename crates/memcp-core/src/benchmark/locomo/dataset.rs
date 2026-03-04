/// LoCoMo dataset loader.
///
/// Parses `locomo10.json` into typed `LoCoMoSample` structs.
/// The file is a JSON array at the top level.

use std::io::BufReader;
use std::path::Path;

use super::LoCoMoSample;

/// Load a LoCoMo dataset from the given path.
///
/// The file must be a JSON array of `LoCoMoSample` objects.
/// Returns an error if the file cannot be opened or parsed.
pub fn load_locomo_dataset(path: &Path) -> Result<Vec<LoCoMoSample>, anyhow::Error> {
    let file = std::fs::File::open(path)
        .map_err(|e| anyhow::anyhow!("Failed to open LoCoMo dataset at {:?}: {}", path, e))?;
    let reader = BufReader::new(file);

    // Attempt top-level array parse first.
    let samples: Vec<LoCoMoSample> = serde_json::from_reader(reader)
        .map_err(|e| anyhow::anyhow!("Failed to parse LoCoMo dataset: {}", e))?;

    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_load_locomo_dataset_missing_file() {
        let result = load_locomo_dataset(Path::new("/nonexistent/locomo10.json"));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Failed to open"), "Error was: {}", err);
    }

    #[test]
    fn test_load_locomo_dataset_valid() {
        let json = r#"[
            {
                "sample_id": "test01",
                "conversation": [
                    {
                        "date": "January 10, 2023",
                        "speakers": ["Alice", "Bob"],
                        "dialog": [
                            {"speaker": "Alice", "dialog_id": 0, "text": "Hey!"},
                            {"speaker": "Bob", "dialog_id": 1, "text": "Hi there!"}
                        ]
                    }
                ],
                "qa": [
                    {
                        "question": "What did Alice say?",
                        "answer": "Hey",
                        "category": 1,
                        "evidence": [0]
                    }
                ]
            }
        ]"#;

        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(json.as_bytes()).unwrap();
        let result = load_locomo_dataset(tmp.path());
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result);
        let samples = result.unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].sample_id, "test01");
    }

    #[test]
    fn test_load_locomo_dataset_invalid_json() {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(b"not valid json").unwrap();
        let result = load_locomo_dataset(tmp.path());
        assert!(result.is_err());
    }
}
