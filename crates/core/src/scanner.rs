// Adapted from Lumid-Off/AirCard-Windows v1.2.2 (MIT).
use regex::Regex;
pub fn is_valid_card_hash(h: &str) -> bool {
    let trimmed = h.trim_matches(['\'', '"']).trim_end_matches(['.', ',']);
    let len = trimmed.len();
    // Real Apple Wallet card hashes are SHA-1 (27-28 chars) or SHA-256 (43-44 chars)
    if len != 27 && len != 28 && len != 43 && len != 44 {
        return false;
    }

    // Must be base64 alphabet characters
    if !trimmed.chars().all(|c| {
        c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '-' || c == '_' || c == '='
    }) {
        return false;
    }

    // '=' can only appear at the end
    if let Some(pos) = trimmed.find('=')
        && pos < len - 2
    {
        return false;
    }

    // Normalize URL-safe base64 and pad
    let mut b64 = trimmed.replace('-', "+").replace('_', "/");
    while !b64.len().is_multiple_of(4) {
        b64.push('=');
    }

    use base64::Engine;
    if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&b64) {
        // Must be exactly 20 bytes (SHA-1) or 32 bytes (SHA-256)
        if decoded.len() == 20 || decoded.len() == 32 {
            // Reject trivial all-identical bytes
            if decoded.iter().all(|&b| b == decoded[0]) {
                return false;
            }

            // Cryptographic hashes have high byte entropy:
            // 1. Must contain both bytes with MSB set (>= 128) and MSB clear (< 128).
            let has_high = decoded.iter().any(|&b| b >= 128);
            let has_low = decoded.iter().any(|&b| b < 128);
            if !has_high || !has_low {
                return false;
            }

            // 2. Must contain at least 12 distinct byte values in 20 bytes
            let mut unique_bytes = std::collections::HashSet::new();
            for &b in &decoded {
                unique_bytes.insert(b);
            }
            if unique_bytes.len() < 12 {
                return false;
            }

            if DUMMY_HASHES.contains(&trimmed)
                || DUMMY_HASHES
                    .iter()
                    .any(|d| d.trim_end_matches('=') == trimmed)
            {
                return false;
            }
            return true;
        }
    }

    false
}

const WALLET_KEYWORDS: &[&str] = &[
    "passd",
    "passbook",
    "passkit",
    "stockholm",
    "nanopassd",
    "npkcompanion",
    "wallet",
    "/cards/",
    "/passes/",
];

const DUMMY_HASHES: &[&str] = &[
    "M6nDwZrkYbFlsodLgCbvyFZQ1cc=",
    "kJL-D0rr-SZhbj2c8nK-OQ9hCMY=",
    "hwAtAmHKYwsQrJbT5cTNDsaxVME=",
];

use std::sync::LazyLock;

static CARD_REGEXES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r"(?i)/(?:Cards|Passes/Cards)/([A-Za-z0-9+/_-]{27,43}=?)(?:\.pkpass|\.cache|\.pkcache|/|\s|\x22|'|\)|,|$)").unwrap(),
        Regex::new(r"/([A-Za-z0-9+/_-]{27,43}=?)\.(?:pkpass|cache|pkcache)").unwrap(),
        Regex::new(r"(?:^|[^A-Za-z0-9+/_=-])((?:[A-Za-z0-9+/_-]{27}|[A-Za-z0-9+/_-]{43})=)(?:$|[^A-Za-z0-9+/_=-])").unwrap(),
        Regex::new(r"(?i)(?:card[_\s]?(?:hash|id)|pass[_\s]?(?:hash|id)|unique[_\s]?id)\s*[:=]\s*['\x22]?([A-Za-z0-9+/_-]{27,43}=?)(?:$|[^A-Za-z0-9+/_=-])").unwrap(),
    ]
});

pub fn extract_card_hashes_from_line(line: &str) -> Vec<String> {
    let lower = line.to_lowercase();
    if !WALLET_KEYWORDS.iter().any(|k| lower.contains(k)) {
        return Vec::new();
    }
    let mut hashes = Vec::new();
    for regex in CARD_REGEXES.iter() {
        for caps in regex.captures_iter(line) {
            let hash = caps.get(1).expect("hash capture").as_str();
            if is_valid_card_hash(hash) {
                let mut normalized = hash.to_string();
                if matches!(normalized.len(), 27 | 43) {
                    normalized.push('=');
                }
                if !hashes.contains(&normalized) {
                    hashes.push(normalized);
                }
            }
        }
    }
    hashes
}

pub fn extract_card_hash_from_line(line: &str) -> Option<String> {
    extract_card_hashes_from_line(line).into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{
        Engine,
        engine::general_purpose::{STANDARD, URL_SAFE},
    };

    #[test]
    fn detects_padded_and_unpadded_sha1_and_sha256_paths_and_labels() {
        for size in [20, 32] {
            let bytes: Vec<_> = (0..size).map(|i| (i * 11) as u8).collect();
            for engine in [STANDARD, URL_SAFE] {
                let hash = engine.encode(&bytes);
                for candidate in [hash.as_str(), hash.trim_end_matches('=')] {
                    for line in [
                        format!("passd /Cards/{candidate}.pkpass"),
                        format!("Wallet card_id='{candidate}'"),
                    ] {
                        assert_eq!(
                            extract_card_hashes_from_line(&line),
                            vec![hash.clone()],
                            "{line}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn does_not_lose_later_candidates_or_reject_url_safe_alphabet() {
        let first = URL_SAFE.encode((0..20).map(|i| (i * 13 + 191) as u8).collect::<Vec<_>>());
        let second = URL_SAFE.encode((0..32).map(|i| (i * 17 + 251) as u8).collect::<Vec<_>>());
        let line = format!(
            "passd /Cards/AAAAAAAAAAAAAAAAAAAAAAAAAAA=.pkpass /Cards/{first}.pkpass /Cards/{second}.pkpass"
        );
        let hashes = extract_card_hashes_from_line(&line);
        assert_eq!(hashes, vec![first, second]);
        assert!(
            extract_card_hashes_from_line("Wallet card_id=AAAAAAAAAAAAAAAAAAAAAAAAAAA=").is_empty()
        );
    }
}
