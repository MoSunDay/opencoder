//! Pure domain functions for the brain capability library: validation,
//! embedding-text composition, and the little-endian f32 byte codec shared
//! with `Store::upsert_brain_vector` / `Store::search_brain_vectors`.
//! No I/O, no state — trivially unit-testable.

use anyhow::{bail, Result};

use crate::types::CapabilityInput;

/// At most this many exemplar inputs per capability.
pub const MAX_ENG_INPUTS: usize = 64;
/// Length cap for `summary` (chars, after trim).
pub const MAX_SUMMARY_LEN: usize = 500;
/// Length cap for `input_desc` / `output_desc` (chars, after trim).
pub const MAX_DESC_LEN: usize = 2000;
/// Length cap for each `eng_inputs` entry (chars, after trim).
pub const MAX_ENG_INPUT_LEN: usize = 4000;

/// Validate a capability payload. Every rule names the offending field in
/// its error message so API consumers get actionable feedback.
pub fn validate(input: &CapabilityInput) -> Result<()> {
    if input.capability_type.trim().is_empty() {
        bail!("capability_type must not be empty");
    }
    if input.summary.trim().is_empty() {
        bail!("summary must not be empty");
    }
    if input.input_desc.trim().is_empty() {
        bail!("input_desc must not be empty");
    }
    if input.output_desc.trim().is_empty() {
        bail!("output_desc must not be empty");
    }
    if input.summary.trim().len() > MAX_SUMMARY_LEN {
        bail!("summary exceeds {MAX_SUMMARY_LEN} chars");
    }
    if input.input_desc.trim().len() > MAX_DESC_LEN {
        bail!("input_desc exceeds {MAX_DESC_LEN} chars");
    }
    if input.output_desc.trim().len() > MAX_DESC_LEN {
        bail!("output_desc exceeds {MAX_DESC_LEN} chars");
    }
    if input.eng_inputs.len() > MAX_ENG_INPUTS {
        bail!("eng_inputs exceeds {MAX_ENG_INPUTS} entries");
    }
    for (i, eng) in input.eng_inputs.iter().enumerate() {
        if eng.trim().is_empty() {
            bail!("eng_inputs[{i}] must not be empty");
        }
        if eng.trim().len() > MAX_ENG_INPUT_LEN {
            bail!("eng_inputs[{i}] exceeds {MAX_ENG_INPUT_LEN} chars");
        }
    }
    Ok(())
}

/// Compose the single text that gets embedded for a capability. Field order
/// is a stable contract (tests assert it): 类型 / 描述 / 输入 / 输出 / 工程输入.
/// Values are trimmed exactly like `validate` checks them, so a stored
/// embedding always matches this composition of the stored fields.
pub fn compose_embed_text(input: &CapabilityInput) -> String {
    let mut lines: Vec<String> = vec![
        format!("类型: {}", input.capability_type.trim()),
        format!("描述: {}", input.summary.trim()),
        format!("输入: {}", input.input_desc.trim()),
        format!("输出: {}", input.output_desc.trim()),
    ];
    if !input.eng_inputs.is_empty() {
        lines.push("工程输入:".to_string());
        for eng in &input.eng_inputs {
            lines.push(format!("- {}", eng.trim()));
        }
    }
    lines.join("\n")
}

/// Encode f32s as little-endian bytes — the blob encoding libsql's bundled
/// `vector32()` accepts directly, so store and query share one codec.
pub fn f32_slice_to_le_bytes(vals: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vals.len() * 4);
    for v in vals {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Decode little-endian f32 bytes. Any length that is not a multiple of 4
/// (the f32 byte width) is rejected rather than silently truncated.
pub fn le_bytes_to_f32_slice(bytes: &[u8]) -> Result<Vec<f32>> {
    if !bytes.len().is_multiple_of(4) {
        bail!(
            "embedding blob length {} is not a multiple of 4 (f32 width)",
            bytes.len()
        );
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> CapabilityInput {
        CapabilityInput {
            capability_type: "tool-usage".into(),
            summary: "can repair failing rust tests".into(),
            input_desc: "a failing test id".into(),
            output_desc: "a green test run".into(),
            eng_inputs: vec!["fix the login test".into()],
        }
    }

    #[test]
    fn validate_accepts_sound_payload() {
        assert!(validate(&base()).is_ok());
    }

    #[test]
    fn validate_rejects_blank_fields_with_field_names() {
        for (field, blank) in [
            ("capability_type", "  "),
            ("summary", "\t "),
            ("input_desc", ""),
            ("output_desc", "   "),
        ] {
            let mut input = base();
            match field {
                "capability_type" => input.capability_type = blank.into(),
                "summary" => input.summary = blank.into(),
                "input_desc" => input.input_desc = blank.into(),
                _ => input.output_desc = blank.into(),
            }
            let err = validate(&input).unwrap_err().to_string();
            assert!(err.contains(field), "expected {field} in error, got: {err}");
        }
    }

    #[test]
    fn validate_enforces_field_length_caps() {
        let cases: [(&str, usize); 4] = [
            ("summary", MAX_SUMMARY_LEN + 1),
            ("input_desc", MAX_DESC_LEN + 1),
            ("output_desc", MAX_DESC_LEN + 1),
            ("eng_inputs[0]", MAX_ENG_INPUT_LEN + 1),
        ];
        for (field, len) in cases {
            let mut input = base();
            let long = "x".repeat(len);
            match field {
                "summary" => input.summary = long,
                "input_desc" => input.input_desc = long,
                "output_desc" => input.output_desc = long,
                _ => input.eng_inputs = vec![long],
            }
            let err = validate(&input).unwrap_err().to_string();
            assert!(err.contains(field), "expected {field} in error, got: {err}");
        }
        // Exactly at the caps still passes.
        let mut input = base();
        input.summary = "s".repeat(MAX_SUMMARY_LEN);
        input.input_desc = "i".repeat(MAX_DESC_LEN);
        input.output_desc = "o".repeat(MAX_DESC_LEN);
        input.eng_inputs = vec!["e".repeat(MAX_ENG_INPUT_LEN)];
        assert!(validate(&input).is_ok());
    }

    #[test]
    fn validate_enforces_eng_input_count() {
        let mut input = base();
        input.eng_inputs = vec!["ok".to_string(); MAX_ENG_INPUTS + 1];
        let err = validate(&input).unwrap_err().to_string();
        assert!(err.contains("eng_inputs"), "got: {err}");
        input.eng_inputs = vec!["ok".to_string(); MAX_ENG_INPUTS];
        assert!(validate(&input).is_ok());
    }

    #[test]
    fn validate_rejects_blank_eng_input_with_index() {
        let mut input = base();
        input.eng_inputs = vec!["keep".into(), "   ".into()];
        let err = validate(&input).unwrap_err().to_string();
        assert!(err.contains("eng_inputs[1]"), "got: {err}");
    }

    #[test]
    fn compose_contains_every_field() {
        let text = compose_embed_text(&base());
        assert!(text.contains("类型: tool-usage"));
        assert!(text.contains("描述: can repair failing rust tests"));
        assert!(text.contains("输入: a failing test id"));
        assert!(text.contains("输出: a green test run"));
        assert!(text.contains("工程输入:"));
        assert!(text.contains("- fix the login test"));
    }

    #[test]
    fn compose_trims_and_omits_empty_eng_inputs_section() {
        let mut input = base();
        input.summary = "  padded  ".into();
        input.eng_inputs.clear();
        let text = compose_embed_text(&input);
        assert!(text.contains("描述: padded"));
        assert!(!text.contains("工程输入"));
    }

    #[test]
    fn le_codec_roundtrips_plain_values() {
        let vals = [0.0f32, -1.5, 42.25, 1e-30, f32::MAX, f32::MIN_POSITIVE];
        let bytes = f32_slice_to_le_bytes(&vals);
        assert_eq!(bytes.len(), vals.len() * 4);
        assert_eq!(le_bytes_to_f32_slice(&bytes).unwrap(), vals.to_vec());
    }

    #[test]
    fn le_codec_preserves_nan_bits() {
        let nan = f32::from_bits(0x7fc0_0001);
        let decoded = le_bytes_to_f32_slice(&f32_slice_to_le_bytes(&[nan])).unwrap();
        assert_eq!(decoded.len(), 1);
        assert!(decoded[0].is_nan());
        assert_eq!(decoded[0].to_bits(), nan.to_bits());
    }

    #[test]
    fn le_codec_rejects_dirty_lengths() {
        for bad in [&[0u8, 0, 0][..], &[0u8; 5][..], &[7u8][..]] {
            assert!(le_bytes_to_f32_slice(bad).is_err(), "len {}", bad.len());
        }
        assert_eq!(le_bytes_to_f32_slice(&[]).unwrap(), Vec::<f32>::new());
    }
}
