//! OSC 52 (terminal clipboard) for the copy path — issue-
//! clipboard-and-selection part 1. `M-w` (`copy_region`) already populates
//! redline's internal kill ring (emacs `kill-ring-save`); this module is
//! the SECOND sink: the terminal-native system clipboard. OSC 52 is chosen
//! over a clipboard crate on purpose — no dependency, and it is the only
//! sink that works over SSH (a clipboard crate cannot reach the remote
//! machine's clipboard through a terminal).
//!
//! Wire format (pinned byte-exact in the tests below):
//! `ESC ] 52 ; c ; <base64(UTF-8)> BEL`, i.e. `\x1b]52;c;<b64>\x07`. The
//! payload is base64 of the region's UTF-8 bytes — that is what makes
//! control bytes (which occur in real source: `\x01` in binary-adjacent
//! files, NULs in paste-quirk buffers) safe to stream through a terminal
//! without corrupting the terminal's state; do not "simplify" it away.
//!
//! Size cap: payloads over [`OSC52_MAX_BYTES`] (32 KiB — the largest
//! single-shot OSC 52 payload commonly honoured) are NOT emitted; the kill
//! ring still gets the full text, and `copy_region`'s message says so.
//! Truncating the clipboard instead of skipping it would copy silently
//! wrong text, which is worse than no clipboard at all.

/// The largest UTF-8 payload `M-w` will encode into an OSC 52 escape.
/// Past this the escape is skipped entirely (kill ring keeps the full
/// text; the copy message reports the skip).
pub const OSC52_MAX_BYTES: usize = 32 * 1024;

/// Build the OSC 52 "copy to clipboard" escape for `text`.
/// `None` when `text`'s UTF-8 byte length exceeds [`OSC52_MAX_BYTES`]
/// (the caller must degrade to the kill ring only and say so).
pub fn osc52_copy(text: &str) -> Option<String> {
    if text.len() > OSC52_MAX_BYTES {
        return None;
    }
    Some(format!("\u{1b}]52;c;{}\u{7}", base64_encode(text.as_bytes())))
}

/// Standard-alphabet base64 (RFC 4640 §4, with `=` padding). Hand-rolled
/// rather than a dependency: OSC 52's "no dependency" requirement is the
/// point of this module, and the encoding is 20 lines.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(n >> 18 & 63) as usize] as char);
        out.push(ALPHABET[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Byte-exact pins for the escape as a whole (the literals were
    /// cross-checked against an independent encoder — the encoding must
    /// not be able to silently rot).
    #[test]
    fn osc52_escape_is_byte_exact() {
        assert_eq!(
            osc52_copy("hello").unwrap(),
            "\u{1b}]52;c;aGVsbG8=\u{7}",
            "ASCII payload"
        );
        // Multi-byte: é (U+00E9 → 0xC3 0xA9), a payload where a wrong
        // byte/char confusion produces DIFFERENT base64.
        assert_eq!(
            osc52_copy("café").unwrap(),
            "\u{1b}]52;c;Y2Fmw6k=\u{7}",
            "UTF-8 payload must be base64 of the BYTES"
        );
        // A CJK-only line (annotation territory): 中 = 0xE4 0xB8 0xAD.
        assert_eq!(
            osc52_copy("中").unwrap(),
            "\u{1b}]52;c;5Lit\u{7}",
        );
        // A mixed multi-byte + trailing newline payload (a whole copied
        // line of annotated source): é, ö, 中, \n.
        assert_eq!(
            osc52_copy("héllo wörld 中\n").unwrap(),
            "\u{1b}]52;c;aMOpbGxvIHfDtnJsZCDkuK0K\u{7}",
        );
    }

    /// Control bytes in the region must survive the encoding verbatim —
    /// this is the safety OSC 52's base64 exists for.
    #[test]
    fn osc52_control_bytes_are_encoded_not_escaped() {
        assert_eq!(
            osc52_copy("\u{1}\u{2}").unwrap(),
            "\u{1b}]52;c;AQI=\u{7}",
            "0x01 0x02 must become base64, never raw control bytes"
        );
    }

    /// Padding shapes: 1 byte → 2 chars + "==", 2 bytes → 3 chars + "=",
    /// 3 bytes → 4 chars (the chunk-boundary cases a naive encoder gets
    /// wrong).
    #[test]
    fn base64_padding_shapes() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"a"), "YQ==");
        assert_eq!(base64_encode(b"ab"), "YWI=");
        assert_eq!(base64_encode(b"abc"), "YWJj");
    }

    /// The cap: exactly OSC52_MAX_BYTES emits, one byte past does not.
    /// (The kill ring is unaffected — checked at the store level.)
    #[test]
    fn osc52_cap_skips_large_payloads() {
        let at_cap = "a".repeat(OSC52_MAX_BYTES);
        assert!(osc52_copy(&at_cap).is_some(), "exactly the cap emits");
        let over = "a".repeat(OSC52_MAX_BYTES + 1);
        assert!(
            osc52_copy(&over).is_none(),
            "one byte past the cap must be skipped, not emitted"
        );
    }
}
