use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

const KEY_BYTES: usize = 1024;

/// Reads one top-level JSON string without retaining unrelated values. The
/// parser follows JSON structure, so key-looking bytes inside strings or
/// nested objects cannot be mistaken for the requested field.
pub(super) fn top_level_text(path: &Path, field: &str, inline_bytes: u64) -> Result<Value> {
    let mut input = JsonInput::new(File::open(path)?);
    input.expect_non_ws(b'{')?;
    if input.peek_non_ws()? == Some(b'}') {
        input.read_non_ws()?;
        return Ok(Value::Null);
    }
    loop {
        let key = input.read_key()?;
        input.expect_non_ws(b':')?;
        if key.as_deref() == Some(field) {
            return input.read_text(field, inline_bytes);
        }
        input.skip_value()?;
        match input.read_non_ws()? {
            Some(b',') => {}
            Some(b'}') => return Ok(Value::Null),
            byte => bail!("expected object delimiter, found {byte:?}"),
        }
    }
}

struct JsonInput<R> {
    reader: BufReader<R>,
    pending: Option<u8>,
    last_string_bytes: u64,
}

impl<R: Read> JsonInput<R> {
    fn new(reader: R) -> Self {
        Self {
            reader: BufReader::with_capacity(8192, reader),
            pending: None,
            last_string_bytes: 0,
        }
    }

    fn read(&mut self) -> Result<Option<u8>> {
        if self.pending.is_some() {
            return Ok(self.pending.take());
        }
        let mut byte = [0u8; 1];
        Ok((self.reader.read(&mut byte)? != 0).then_some(byte[0]))
    }

    fn unread(&mut self, byte: u8) {
        debug_assert!(self.pending.is_none());
        self.pending = Some(byte);
    }

    fn read_non_ws(&mut self) -> Result<Option<u8>> {
        loop {
            match self.read()? {
                Some(byte) if byte.is_ascii_whitespace() => {}
                byte => return Ok(byte),
            }
        }
    }

    fn peek_non_ws(&mut self) -> Result<Option<u8>> {
        let byte = self.read_non_ws()?;
        if let Some(byte) = byte {
            self.unread(byte);
        }
        Ok(byte)
    }

    fn expect_non_ws(&mut self, expected: u8) -> Result<()> {
        let actual = self.read_non_ws()?;
        if actual != Some(expected) {
            bail!(
                "expected JSON byte {:?}, found {actual:?}",
                expected as char
            );
        }
        Ok(())
    }

    fn read_key(&mut self) -> Result<Option<String>> {
        if self.read_non_ws()? != Some(b'"') {
            bail!("expected JSON object key");
        }
        let raw = self.read_string_raw(KEY_BYTES as u64)?;
        match raw {
            Some(raw) => serde_json::from_slice(&raw)
                .map(Some)
                .context("decode JSON object key"),
            None => Ok(None),
        }
    }

    fn read_text(&mut self, field: &str, inline_bytes: u64) -> Result<Value> {
        match self.read_non_ws()? {
            Some(b'n') => {
                self.expect_bytes(b"ull")?;
                Ok(Value::Null)
            }
            Some(b'"') => match self.read_string_raw(inline_bytes)? {
                Some(raw) => serde_json::from_slice(&raw).context("decode bounded topic field"),
                None => Ok(json!({
                    "omitted": true,
                    "total_bytes": self.last_string_bytes,
                    "read_via": "detail_field",
                    "field": "team.topic",
                })),
            },
            _ => bail!("{field} is not a JSON string or null"),
        }
    }

    fn expect_bytes(&mut self, expected: &[u8]) -> Result<()> {
        for expected in expected {
            if self.read()? != Some(*expected) {
                bail!("invalid JSON literal");
            }
        }
        Ok(())
    }

    // Includes both quote bytes. `None` means the string exceeded `limit`;
    // `last_string_bytes` still records its complete encoded length.
    fn read_string_raw(&mut self, limit: u64) -> Result<Option<Vec<u8>>> {
        let mut raw = Vec::with_capacity((limit as usize).min(KEY_BYTES) + 1);
        raw.push(b'"');
        let mut total = 1u64;
        let mut escaped = false;
        loop {
            let byte = self.read()?.context("unterminated JSON string")?;
            total += 1;
            if total <= limit {
                raw.push(byte);
            }
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                break;
            }
        }
        self.last_string_bytes = total;
        Ok((total <= limit).then_some(raw))
    }

    fn skip_value(&mut self) -> Result<()> {
        match self.read_non_ws()? {
            Some(b'"') => {
                self.read_string_raw(0)?;
            }
            Some(b'{') => self.skip_object()?,
            Some(b'[') => self.skip_array()?,
            Some(b't') => self.expect_bytes(b"rue")?,
            Some(b'f') => self.expect_bytes(b"alse")?,
            Some(b'n') => self.expect_bytes(b"ull")?,
            Some(byte @ (b'-' | b'0'..=b'9')) => self.skip_number(byte)?,
            byte => bail!("invalid JSON value start {byte:?}"),
        }
        Ok(())
    }

    fn skip_object(&mut self) -> Result<()> {
        if self.peek_non_ws()? == Some(b'}') {
            self.read_non_ws()?;
            return Ok(());
        }
        loop {
            self.read_key()?;
            self.expect_non_ws(b':')?;
            self.skip_value()?;
            match self.read_non_ws()? {
                Some(b',') => {}
                Some(b'}') => return Ok(()),
                byte => bail!("invalid object delimiter {byte:?}"),
            }
        }
    }

    fn skip_array(&mut self) -> Result<()> {
        if self.peek_non_ws()? == Some(b']') {
            self.read_non_ws()?;
            return Ok(());
        }
        loop {
            self.skip_value()?;
            match self.read_non_ws()? {
                Some(b',') => {}
                Some(b']') => return Ok(()),
                byte => bail!("invalid array delimiter {byte:?}"),
            }
        }
    }

    fn skip_number(&mut self, _first: u8) -> Result<()> {
        while let Some(byte) = self.read()? {
            if matches!(byte, b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                continue;
            }
            self.unread(byte);
            break;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn top_level_field_ignores_key_like_text_and_nested_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("topic.json");
        let mut file = File::create(&path).unwrap();
        write!(
            file,
            "{}",
            serde_json::to_string(&json!({
                "title": "x\"requirement\":\"spoof\"",
                "nested": {"requirement": "nested"},
                "requirement": "real \"final_summary\": \"spoof\"",
                "final_summary": "actual",
            }))
            .unwrap()
        )
        .unwrap();
        assert_eq!(
            top_level_text(&path, "requirement", 64).unwrap(),
            "real \"final_summary\": \"spoof\""
        );
        assert_eq!(
            top_level_text(&path, "final_summary", 64).unwrap(),
            "actual"
        );
    }
}
