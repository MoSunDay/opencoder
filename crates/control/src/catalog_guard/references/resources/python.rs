//! Bounded CPython 3.11 marshal reader. It extracts strings without loading or
//! executing code objects. Other cache versions and unknown tags stay closed.
//! Format: https://github.com/python/cpython/blob/3.11/Python/marshal.c
use anyhow::{bail, ensure, Context, Result};

pub(super) fn strings(bytes: &[u8]) -> Result<Vec<String>> {
    ensure!(bytes.len() >= 17, "truncated Python cache");
    let flags = u32::from_le_bytes(bytes[4..8].try_into()?);
    ensure!(flags & !3 == 0, "unknown Python cache flags");
    ensure!(bytes[16] & 0x7f == b'c', "Python cache has no code object");
    let mut reader = Reader {
        bytes: &bytes[16..],
        offset: 0,
        refs: 0,
        objects: 0,
        strings: Vec::new(),
    };
    ensure!(!reader.object(0)?, "null Python cache");
    ensure!(
        reader.offset == reader.bytes.len(),
        "trailing Python cache data"
    );
    Ok(reader.strings)
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
    refs: u32,
    objects: usize,
    strings: Vec<String>,
}

impl<'a> Reader<'a> {
    fn take(&mut self, size: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(size)
            .context("Python cache length overflow")?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .context("truncated Python cache")?;
        self.offset = end;
        Ok(bytes)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn integer(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn length(&mut self) -> Result<usize> {
        usize::try_from(self.integer()?).context("negative Python cache length")
    }
    fn sequence(&mut self, size: usize, depth: usize) -> Result<()> {
        ensure!(
            size <= self.bytes.len(),
            "Python cache sequence limit exceeded"
        );
        for _ in 0..size {
            ensure!(!self.object(depth + 1)?, "unexpected null object");
        }
        Ok(())
    }
    // True denotes only the dictionary terminator, not the Python None object.
    fn object(&mut self, depth: usize) -> Result<bool> {
        ensure!(
            depth <= 64 && self.objects < 100_000,
            "Python cache object limit exceeded"
        );
        self.objects += 1;
        let tag = self.byte()?;
        if tag & 0x80 != 0 {
            self.refs += 1;
        }
        match tag & 0x7f {
            b'0' => return Ok(true),
            b'N' | b'F' | b'T' | b'S' | b'.' => {}
            b'i' => {
                self.take(4)?;
            }
            b'I' | b'g' => {
                self.take(8)?;
            }
            b'y' => {
                self.take(16)?;
            }
            b'f' | b'x' => {
                for _ in 0..if tag & 0x7f == b'x' { 2 } else { 1 } {
                    let size = self.byte()? as usize;
                    self.take(size)?;
                }
            }
            b'l' => {
                let size = self.integer()?.unsigned_abs() as usize;
                self.take(
                    size.checked_mul(2)
                        .context("Python integer size overflow")?,
                )?;
            }
            b's' | b'u' | b't' | b'a' | b'A' | b'z' | b'Z' => {
                let size = if matches!(tag & 0x7f, b'z' | b'Z') {
                    self.byte()? as usize
                } else {
                    self.length()?
                };
                let bytes = self.take(size)?;
                let text = if tag & 0x7f == b's' {
                    // Bytecode and byte constants use this tag; replacement
                    // characters separate binary bytes without losing ASCII names.
                    String::from_utf8_lossy(bytes).into_owned()
                } else {
                    std::str::from_utf8(bytes)
                        .context("invalid Python string")?
                        .to_owned()
                };
                self.strings.push(text);
            }
            b'(' | b'[' | b'<' | b'>' => {
                let size = self.length()?;
                self.sequence(size, depth)?;
            }
            b')' => {
                let size = self.byte()? as usize;
                self.sequence(size, depth)?;
            }
            b'{' => {
                while !self.object(depth + 1)? {
                    ensure!(!self.object(depth + 1)?, "null dictionary value");
                }
            }
            b'r' => {
                let index = self.integer()?;
                ensure!(
                    index >= 0 && (index as u32) < self.refs,
                    "invalid Python object reference"
                );
            }
            b'c' => {
                self.take(5 * 4)?; // argument counts, stack size and flags
                self.sequence(8, depth)?; // code, constants, names, locals and identity strings
                self.take(4)?; // first line
                self.sequence(2, depth)?; // line and exception tables
            }
            _ => bail!("unsupported Python cache object"),
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn nested_python_code_constants_and_numeric_values_are_inspected_without_execution() {
        let bytes = base64::engine::general_purpose::STANDARD.decode("pw0NCgAAAAAAAAAAAAAAAGMAAAAAAAAAAAAAAAABAAAAAAAAAPMMAAAAlwBkAIQAWgBkAVMAKQJjAAAAAAAAAAAAAAAAAwAAAAMAAADzDAAAAJcAZAFkAmQDnAJTACkETnoPb3BlcmF0b3Itc2FtcGxlKQVnAAAAAAAADEB5AAAAAAAAAAAAAAAAAADwP2wGAAAAAAAAAAAAAAAAACAAcwwAAABhZ2VudC1zYW1wbGV1CwAAAHNub3dtYW4g4piDKQLaBnRhcmdldNoGdmFsdWVzqQByBAAAAPMAAAAA+gpmaXh0dXJlLnB52gZoZWxwZXJyBwAAAAEAAABzEQAAAIAA2BMk0DB00Ah10Ah10AF1cgUAAABOKQFyBwAAAHIEAAAAcgUAAAByBgAAAPoIPG1vZHVsZT5yCAAAAAEAAABzIwAAAPADAQEB8AIBAXYB8AABAXYB8AABAXYB8AABAXYB8AABAXYBcgUAAAA=").unwrap();
        let values = strings(&bytes).unwrap();
        assert!(values.iter().any(|value| value == "operator-sample"));
        assert!(values.iter().any(|value| value == "agent-sample"));
        assert!(values.iter().any(|value| value == "snowman ☃"));
        for end in 0..bytes.len() {
            assert!(strings(&bytes[..end]).is_err());
        }
    }
}
