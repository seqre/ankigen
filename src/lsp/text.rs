//! Byte offset ⇄ LSP position conversion.
//!
//! The parser works in byte offsets; LSP positions are a line plus a column
//! counted in the negotiated encoding (UTF-8 bytes or UTF-16 code units).

use std::ops::Range;

use lsp_types::Position;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16,
}

/// The byte offset at which each line of a text starts.
pub struct LineIndex {
    line_starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        LineIndex { line_starts }
    }

    /// The position of byte `offset` (which must lie on a char boundary).
    pub fn position(&self, text: &str, offset: usize, enc: Encoding) -> Position {
        let line = self.line_starts.partition_point(|&s| s <= offset) - 1;
        let start = self.line_starts[line];
        let character = match enc {
            Encoding::Utf8 => offset - start,
            Encoding::Utf16 => text[start..offset].encode_utf16().count(),
        };
        Position::new(line as u32, character as u32)
    }

    pub fn range(&self, text: &str, range: Range<usize>, enc: Encoding) -> lsp_types::Range {
        lsp_types::Range::new(
            self.position(text, range.start, enc),
            self.position(text, range.end, enc),
        )
    }

    /// The byte offset of `pos`, clamped to the end of its line (or of the
    /// text) when the client points past it.
    pub fn offset(&self, text: &str, pos: Position, enc: Encoding) -> usize {
        let Some(&start) = self.line_starts.get(pos.line as usize) else {
            return text.len();
        };
        let line = text[start..].split('\n').next().unwrap_or("");
        let line = line.strip_suffix('\r').unwrap_or(line);
        let want = pos.character as usize;
        let column = match enc {
            Encoding::Utf8 => {
                let mut c = want.min(line.len());
                while !line.is_char_boundary(c) {
                    c -= 1;
                }
                c
            }
            Encoding::Utf16 => {
                let mut units = 0;
                let mut bytes = 0;
                for ch in line.chars() {
                    if units + ch.len_utf16() > want {
                        break;
                    }
                    units += ch.len_utf16();
                    bytes += ch.len_utf8();
                }
                bytes
            }
        };
        start + column
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_both_ways_in_both_encodings() {
        let text = "q: España 🎉 x\r\na: ok\n";
        let index = LineIndex::new(text);
        let x = text.find('x').unwrap();
        for (enc, col) in [(Encoding::Utf8, 16), (Encoding::Utf16, 13)] {
            let pos = index.position(text, x, enc);
            assert_eq!(pos, Position::new(0, col));
            assert_eq!(index.offset(text, pos, enc), x);
        }
        let a = text.find("a:").unwrap();
        assert_eq!(
            index.position(text, a, Encoding::Utf16),
            Position::new(1, 0)
        );
        // Past the end of a line or of the text clamps.
        assert_eq!(
            index.offset(text, Position::new(1, 99), Encoding::Utf16),
            a + 5
        );
        assert_eq!(
            index.offset(text, Position::new(9, 0), Encoding::Utf8),
            text.len()
        );
    }
}
