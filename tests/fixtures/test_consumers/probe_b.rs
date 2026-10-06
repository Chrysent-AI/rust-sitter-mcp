const MAX_PROMPT_BYTES: usize = 4;
const MAX_MODEL_BYTES: usize = 8;
const MAX_EVIDENCE: usize = 16;
const MAX_KIND_BYTES: usize = 32;

fn limits() -> usize {
    MAX_PROMPT_BYTES + MAX_MODEL_BYTES + MAX_EVIDENCE + MAX_KIND_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt() {
        let oversized = "x".repeat(MAX_PROMPT_BYTES + 1);
        assert_eq!(oversized.len(), MAX_PROMPT_BYTES + 1);
    }

    #[test]
    fn model() {
        let oversized = "x".repeat(MAX_MODEL_BYTES + 1);
        assert_eq!(oversized.len(), MAX_MODEL_BYTES + 1);
    }

    #[test]
    fn evidence() {
        let oversized = "x".repeat(MAX_EVIDENCE + 1);
        assert_eq!(oversized.len(), MAX_EVIDENCE + 1);
    }

    #[test]
    fn kind() {
        let oversized = "x".repeat(MAX_KIND_BYTES + 1);
        assert_eq!(oversized.len(), MAX_KIND_BYTES + 1);
    }
}
