//! Byte-metered JSONL frame reader. Enforces the single-frame limit WHILE
//! reading (memory stays bounded by the cap + one buffer chunk), counts the
//! full oversized line, and drains it so the pump can continue. Same
//! mechanism at the 16 MiB production default and at scaled test thresholds.

use std::io::{self, BufRead};

#[derive(Debug, PartialEq, Eq)]
pub enum LineOutcome {
    /// Complete line bytes (newline excluded).
    Line(Vec<u8>),
    /// Line exceeded the cap; `bytes` is the full line size (newline excluded).
    /// The payload was discarded, never parsed, never treated as success.
    Oversize { bytes: usize },
    /// Clean end of stream.
    Eof,
}

pub fn read_line_metered<R: BufRead>(reader: &mut R, cap: usize) -> io::Result<LineOutcome> {
    let mut buf: Vec<u8> = Vec::new();
    let mut total: usize = 0;
    let mut oversize = false;
    loop {
        let available = match reader.fill_buf() {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if available.is_empty() {
            // EOF: a trailing partial line still counts as a frame.
            if total == 0 {
                return Ok(LineOutcome::Eof);
            }
            return Ok(if oversize {
                LineOutcome::Oversize { bytes: total }
            } else {
                LineOutcome::Line(buf)
            });
        }
        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            total += pos;
            if !oversize {
                if total > cap {
                    oversize = true;
                    buf.clear();
                    buf.shrink_to_fit();
                } else {
                    buf.extend_from_slice(&available[..pos]);
                }
            }
            reader.consume(pos + 1);
            return Ok(if oversize {
                LineOutcome::Oversize { bytes: total }
            } else {
                LineOutcome::Line(buf)
            });
        }
        total += available.len();
        if !oversize {
            if total > cap {
                oversize = true;
                buf.clear();
                buf.shrink_to_fit();
            } else {
                buf.extend_from_slice(available);
            }
        }
        let n = available.len();
        reader.consume(n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    #[test]
    fn small_line_returned_verbatim() {
        let mut r = BufReader::new(&b"{\"a\":1}\nnext\n"[..]);
        assert_eq!(
            read_line_metered(&mut r, 1024).unwrap(),
            LineOutcome::Line(b"{\"a\":1}".to_vec())
        );
        assert_eq!(
            read_line_metered(&mut r, 1024).unwrap(),
            LineOutcome::Line(b"next".to_vec())
        );
        assert_eq!(read_line_metered(&mut r, 1024).unwrap(), LineOutcome::Eof);
    }

    #[test]
    fn oversize_line_metered_and_drained() {
        let data = format!("{}\nsmall\n", "x".repeat(5000));
        let mut r = BufReader::new(data.as_bytes());
        assert_eq!(
            read_line_metered(&mut r, 1024).unwrap(),
            LineOutcome::Oversize { bytes: 5000 }
        );
        // Pump continues after the violation.
        assert_eq!(
            read_line_metered(&mut r, 1024).unwrap(),
            LineOutcome::Line(b"small".to_vec())
        );
    }

    #[test]
    fn exact_cap_is_allowed() {
        let data = format!("{}\n", "y".repeat(1024));
        let mut r = BufReader::new(data.as_bytes());
        assert_eq!(
            read_line_metered(&mut r, 1024).unwrap(),
            LineOutcome::Line(vec![b'y'; 1024])
        );
    }

    #[test]
    fn partial_line_at_eof_is_a_frame() {
        let mut r = BufReader::new(&b"tail"[..]);
        assert_eq!(
            read_line_metered(&mut r, 1024).unwrap(),
            LineOutcome::Line(b"tail".to_vec())
        );
        assert_eq!(read_line_metered(&mut r, 1024).unwrap(), LineOutcome::Eof);
    }
}
