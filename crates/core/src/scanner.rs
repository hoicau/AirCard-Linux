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

    // Reject strings with multiple underscores or hyphens (typical of system asset/bundle names)
    if trimmed.chars().filter(|&c| c == '_').count() > 1
        || trimmed.chars().filter(|&c| c == '-').count() > 2
    {
        return false;
    }

    // Reject obvious system identifiers, bundle IDs and common keywords
    let lower = trimmed.to_lowercase();
    if lower.contains("mobileasset")
        || lower.contains("com_apple")
        || lower.contains("com.")
        || lower.contains("apple.")
        || lower.contains("curtain")
        || lower.contains("binder")
        || lower.contains("optimizer")
        || lower.contains("system")
        || lower.contains("uaf")
        || lower.contains("siri")
        || lower.contains("dialog")
        || lower.contains("planner")
        || lower.contains("linguistic")
        || lower.contains("timing")
        || lower.contains("model")
        || lower.contains("translation")
        || lower.contains("visual")
        || lower.contains("device")
        || lower.contains("override")
        || lower.contains("motion")
        || lower.contains("search")
    {
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
        Regex::new(r"/(?:Cards|Passes/Cards)/([A-Za-z0-9+/_-]{27,44})(?:\.pkpass|\.cache|\.pkcache|/|\s|\x22|'|\)|,|$)").unwrap(),
        Regex::new(r"/([A-Za-z0-9+/_-]{27,44})\.(?:pkpass|cache|pkcache)").unwrap(),
        Regex::new(r"(?:^|[^A-Za-z0-9+/_-])([A-Za-z0-9+/_-]{27}=)(?:$|[^A-Za-z0-9+/_-])").unwrap(),
        Regex::new(r"(?i)(?:card[_\s]?(?:hash|id)|pass[_\s]?(?:hash|id)|unique[_\s]?id)\s*[:=]\s*['\x22]?([A-Za-z0-9+=_-]{27,44})").unwrap(),
    ]
});

pub fn extract_card_hash_from_line(line: &str) -> Option<String> {
    let lower = line.to_lowercase();
    let has_wallet = WALLET_KEYWORDS.iter().any(|k| lower.contains(k));
    if !has_wallet {
        return None;
    }

    for r in CARD_REGEXES.iter() {
        if let Some(caps) = r.captures(line)
            && let Some(m) = caps.get(1)
        {
            let h = m
                .as_str()
                .trim()
                .trim_matches(['\'', '"'])
                .trim_end_matches(['.', ',']);
            if is_valid_card_hash(h) {
                let mut norm = h.to_string();
                if norm.len() == 27 {
                    norm.push('=');
                }
                return Some(norm);
            }
        }
    }
    None
}
