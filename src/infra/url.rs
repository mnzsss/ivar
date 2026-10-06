use std::fmt::Write as _;

/// Percent-encode a URI component per RFC 3986 (space as `%20`) or
/// `application/x-www-form-urlencoded` (space as `+`).
#[must_use]
pub fn encode_component(s: &str, space_as_plus: bool) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for &byte in s.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' if space_as_plus => out.push('+'),
            b' ' => out.push_str("%20"),
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "../../tests/unit/infra/url.rs"]
mod tests;
