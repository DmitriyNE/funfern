#[cfg(any(target_arch = "wasm32", test))]
use flate2::read::ZlibDecoder;
use flate2::{Compression, write::ZlibEncoder};
use funfern_app::{editor::Document, persistence};
#[cfg(any(target_arch = "wasm32", test))]
use std::io::Read;
use std::io::Write;

const PREFIX: &str = "scene=v1.";
const MAX_FRAGMENT_CHARS: usize = 128 * 1024;
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn encode(document: &Document) -> Result<String, String> {
    let json = persistence::save_compact(document)?;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder
        .write_all(&json)
        .map_err(|error| error.to_string())?;
    let compressed = encoder.finish().map_err(|error| error.to_string())?;
    let payload = base64_encode(&compressed);
    if PREFIX.len() + payload.len() > MAX_FRAGMENT_CHARS {
        return Err("Scene is too large for a shareable link; save it as a file instead".into());
    }
    Ok(format!("{PREFIX}{payload}"))
}

#[cfg(any(target_arch = "wasm32", test))]
pub fn decode(fragment: &str) -> Result<Vec<u8>, String> {
    let fragment = fragment.strip_prefix('#').unwrap_or(fragment);
    if fragment.len() > MAX_FRAGMENT_CHARS {
        return Err("Shared scene fragment is too large".into());
    }
    let payload = fragment
        .strip_prefix(PREFIX)
        .ok_or("Unsupported shared scene link")?;
    let compressed = base64_decode(payload)?;
    let mut decoder =
        ZlibDecoder::new(compressed.as_slice()).take((persistence::MAX_FILE_BYTES + 1) as u64);
    let mut json = Vec::new();
    decoder
        .read_to_end(&mut json)
        .map_err(|_| "Shared scene data is damaged".to_string())?;
    if json.len() > persistence::MAX_FILE_BYTES {
        return Err("Shared scene expands beyond 2 MiB".into());
    }
    Ok(json)
}

#[cfg(target_arch = "wasm32")]
pub fn initial_fragment() -> Option<Result<Vec<u8>, String>> {
    let fragment = web_sys::window()?.location().hash().ok()?;
    fragment
        .strip_prefix('#')
        .is_some_and(|value| value.starts_with("scene="))
        .then(|| decode(&fragment))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn initial_fragment() -> Option<Result<Vec<u8>, String>> {
    None
}

#[cfg(target_arch = "wasm32")]
pub fn link(fragment: &str) -> Result<String, String> {
    let location = web_sys::window()
        .ok_or("Browser window is unavailable")?
        .location();
    location
        .set_hash(fragment)
        .map_err(|_| "Could not update the browser address".to_string())?;
    location
        .href()
        .map_err(|_| "Could not read the browser address".to_string())
}

#[cfg(target_arch = "wasm32")]
pub fn replace_fragment(fragment: &str) -> Result<(), String> {
    let window = web_sys::window().ok_or("Browser window is unavailable")?;
    window
        .history()
        .map_err(|_| "Browser history is unavailable".to_string())?
        .replace_state_with_url(
            &wasm_bindgen::JsValue::NULL,
            "",
            Some(&format!("#{fragment}")),
        )
        .map_err(|_| "Could not update the shared scene address".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn link(fragment: &str) -> Result<String, String> {
    Ok(format!("https://dmitriyne.github.io/funfern/#{fragment}"))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn replace_fragment(_fragment: &str) -> Result<(), String> {
    Ok(())
}

fn base64_encode(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        result.push(ALPHABET[((value >> 18) & 63) as usize] as char);
        result.push(ALPHABET[((value >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            result.push(ALPHABET[((value >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            result.push(ALPHABET[(value & 63) as usize] as char);
        }
    }
    result
}

#[cfg(any(target_arch = "wasm32", test))]
fn base64_decode(text: &str) -> Result<Vec<u8>, String> {
    if text.is_empty() || text.len() % 4 == 1 || !text.is_ascii() {
        return Err("Shared scene encoding is invalid".into());
    }
    let mut result = Vec::with_capacity(text.len() / 4 * 3 + 2);
    for chunk in text.as_bytes().chunks(4) {
        let mut value = 0_u32;
        for byte in chunk {
            let digit = match byte {
                b'A'..=b'Z' => byte - b'A',
                b'a'..=b'z' => byte - b'a' + 26,
                b'0'..=b'9' => byte - b'0' + 52,
                b'-' => 62,
                b'_' => 63,
                _ => return Err("Shared scene encoding is invalid".into()),
            };
            value = (value << 6) | u32::from(digit);
        }
        value <<= 6 * (4 - chunk.len());
        result.push((value >> 16) as u8);
        if chunk.len() > 2 {
            result.push((value >> 8) as u8);
        }
        if chunk.len() > 3 {
            result.push(value as u8);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragment_round_trip_is_compact_and_url_safe() {
        let document = Document::default();
        let fragment = encode(&document).unwrap();
        assert!(fragment.starts_with(PREFIX));
        assert!(
            fragment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric()
                    || matches!(byte, b'=' | b'.' | b'-' | b'_'))
        );
        let bytes = decode(&fragment).unwrap();
        let mut candidate = persistence::parse(&bytes).unwrap();
        assert_eq!(candidate.advance(100_000).unwrap().unwrap(), document);
    }

    #[test]
    fn damaged_fragments_are_rejected() {
        for fragment in ["scene=v2.abc", "scene=v1.a", "scene=v1.%%%%"] {
            assert!(decode(fragment).is_err());
        }
    }
}
