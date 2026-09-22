//! Offline .passthm conversion adapted from AirCard-Windows v1.2.2 (MIT).
use crate::assets::{AssetError, Resource, Result, decode_image, encode_png};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read},
};

pub const MAX_ARCHIVE_BYTES: usize = 32 * 1024 * 1024;
const MAX_ENTRY_BYTES: usize = 4 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
const EN: [&str; 10] = [
    "+", "", "A B C", "D E F", "G H I", "J K L", "M N O", "P Q R S", "T U V", "W X Y Z",
];
const RU: [&str; 10] = [
    "+",
    "",
    "А Б В Г",
    "Д Е Ж З",
    "И Й К Л",
    "М Н О П",
    "Р С Т У",
    "Ф Х Ц Ч",
    "Ш Щ Ъ Ы",
    "Ь Э Ю Я",
];
const UK: [&str; 10] = [
    "+",
    "",
    "А Б В Г Ґ",
    "Д Е Є Ж З",
    "И І Ї Й",
    "К Л М Н",
    "О П Р С",
    "Т У Ф Х",
    "Ц Ч Ш Щ",
    "Ь Ю Я",
];
#[derive(Debug, Clone, Copy)]
pub enum Language {
    En,
    Ru,
    Uk,
    Ja,
    All,
}
#[derive(Debug, Clone, Copy)]
pub enum TelephonyVersion {
    V8,
    V9,
    V10,
}
impl TelephonyVersion {
    pub fn directory(self) -> &'static str {
        match self {
            Self::V8 => "TelephonyUI-8",
            Self::V9 => "TelephonyUI-9",
            Self::V10 => "TelephonyUI-10",
        }
    }
}
pub struct Theme {
    pub detected_versions: BTreeSet<String>,
    pub target_version: TelephonyVersion,
    pub resources: Vec<Resource>,
    pub previews: BTreeMap<String, Vec<u8>>,
}
// Validate the ordinary EOCD before ZipArchive allocates its entry index. ZIP64,
// split archives and trailing data are deliberately outside this small theme format.
fn preflight(bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_ARCHIVE_BYTES || bytes.len() < 22 {
        return Err(AssetError::Limit);
    }
    let start = bytes.len().saturating_sub(65557);
    let end = (start..=bytes.len() - 22)
        .rev()
        .find(|&p| {
            bytes[p..].starts_with(b"PK\x05\x06")
                && p + 22 + usize::from(u16::from_le_bytes([bytes[p + 20], bytes[p + 21]]))
                    == bytes.len()
        })
        .ok_or(AssetError::Invalid("missing ZIP end record"))?;
    let short = |p| u16::from_le_bytes([bytes[end + p], bytes[end + p + 1]]);
    let long = |p| u32::from_le_bytes(bytes[end + p..end + p + 4].try_into().unwrap());
    if short(4) != 0
        || short(6) != 0
        || short(8) != short(10)
        || short(10) > 256
        || short(10) == 0
        || long(12) == u32::MAX
        || long(16) == u32::MAX
        || u64::from(long(12)) + u64::from(long(16)) != end as u64
    {
        return Err(AssetError::Invalid("unsupported ZIP layout or entry count"));
    }
    Ok(())
}
pub fn parse(
    bytes: &[u8],
    target: TelephonyVersion,
    language: Language,
    bold: bool,
) -> Result<Theme> {
    preflight(bytes)?;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    if archive.len() > 256 {
        return Err(AssetError::Limit);
    }
    let mut names = BTreeSet::new();
    let mut expanded = 0u64;
    let mut versions = BTreeSet::new();
    // Validate every entry, including ignored dotfiles, before decoding any image.
    for i in 0..archive.len() {
        let file = archive.by_index(i)?;
        let name = std::str::from_utf8(file.name_raw())
            .map_err(|_| AssetError::Invalid("non-UTF8 archive path"))?;
        let path = name.strip_suffix('/').unwrap_or(name);
        crate::safe_relative_path(path).map_err(|_| AssetError::Invalid("unsafe archive path"))?;
        for part in path.split('/') {
            crate::safe_leaf(part).map_err(|_| AssetError::Invalid("oversize archive leaf"))?;
        }
        if !names.insert(path.to_string()) {
            return Err(AssetError::Invalid("duplicate archive path"));
        }
        let mode = file.unix_mode().unwrap_or(0) & 0o170000;
        if file.encrypted()
            || !matches!(mode, 0 | 0o100000 | 0o040000)
            || (mode == 0o040000 && !file.is_dir())
        {
            return Err(AssetError::Invalid(
                "encrypted or non-regular archive entry",
            ));
        }
        expanded = expanded.checked_add(file.size()).ok_or(AssetError::Limit)?;
        if file.size() > MAX_ENTRY_BYTES as u64 || expanded > MAX_ARCHIVE_BYTES as u64 {
            return Err(AssetError::Limit);
        }
        for part in path.to_ascii_lowercase().split('/') {
            for version in [8, 9, 10] {
                if part == format!("telephonyui-{version}")
                    || part == format!("telephony-{version}")
                {
                    versions.insert(format!("TelephonyUI-{version}"));
                }
            }
        }
    }
    let digit_re = regex::Regex::new(r"^(?:[a-zA-Z]+-)?([0-9*#])(?:-([^-\n]+))?").unwrap();
    let strip = regex::Regex::new(r"(?i)--?white(?:-bold)?$").unwrap();
    let mut items = BTreeMap::<String, Vec<u8>>::new();
    let mut previews = BTreeMap::new();
    let mut output_bytes = 0usize;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_string();
        if file.is_dir()
            || name
                .split('/')
                .any(|p| p.starts_with('.') || p == "__MACOSX")
        {
            continue;
        }
        let leaf = name.rsplit('/').next().unwrap();
        let Some((stem, ext)) = leaf.rsplit_once('.') else {
            continue;
        };
        if !["png", "jpg", "jpeg", "webp"].contains(&ext.to_ascii_lowercase().as_str()) {
            continue;
        }
        let clean = strip.replace(stem, "");
        let Some(caps) = digit_re.captures(&clean) else {
            continue;
        };
        let digit = caps.get(1).unwrap().as_str();
        let custom = caps
            .get(2)
            .map(|s| s.as_str().trim())
            .filter(|s| !s.is_empty());
        let mut data = Vec::new();
        file.by_ref()
            .take((MAX_ENTRY_BYTES + 1) as u64)
            .read_to_end(&mut data)?;
        if data.len() > MAX_ENTRY_BYTES || data.len() as u64 != file.size() {
            return Err(AssetError::Limit);
        }
        let image = decode_image(&data)?;
        if image.width() > 2048 || image.height() > 2048 {
            return Err(AssetError::Limit);
        }
        let png = encode_png(&image)?;
        if png.len() > MAX_ENTRY_BYTES {
            return Err(AssetError::Limit);
        }
        if let Some(previous) = previews.get(digit) {
            if previous != &png {
                return Err(AssetError::Invalid("conflicting images for one keypad key"));
            }
        } else {
            previews.insert(digit.to_string(), png.clone());
        }
        let n = digit.parse::<usize>().ok();
        let english = n.map(|n| EN[n]).unwrap_or("");
        let prefixes: &[&str] = match language {
            Language::En => &["en", "other"],
            Language::Ru => &["ru", "other", "en"],
            Language::Uk => &["uk", "other", "en"],
            Language::Ja => &["ja", "other", "en"],
            Language::All => &[
                "en", "other", "ru", "uk", "ja", "es", "fr", "de", "it", "pt", "tr", "pl", "ko",
                "zh",
            ],
        };
        let suffix = if bold { "-bold" } else { "" };
        let mut add = |prefix: &str, sub: &str| -> Result<()> {
            for sub in BTreeSet::from([sub.to_string(), sub.replace(' ', "")]) {
                let leaf = format!("{prefix}-{digit}-{sub}--white{suffix}.png");
                crate::safe_leaf(&leaf)
                    .map_err(|_| AssetError::Invalid("unsafe generated resource name"))?;
                if !items.contains_key(&leaf) {
                    output_bytes = output_bytes
                        .checked_add(png.len())
                        .ok_or(AssetError::Limit)?;
                    if output_bytes > MAX_OUTPUT_BYTES || items.len() >= 2047 {
                        return Err(AssetError::Limit);
                    }
                    items.insert(leaf, png.clone());
                }
            }
            Ok(())
        };
        for prefix in prefixes {
            add(prefix, "")?;
            add(prefix, english)?;
            let localized = match language {
                Language::Ru => n.map(|n| RU[n]),
                Language::Uk => n.map(|n| UK[n]),
                Language::All if *prefix == "ru" => n.map(|n| RU[n]),
                Language::All if *prefix == "uk" => n.map(|n| UK[n]),
                _ => None,
            };
            if let Some(sub) = localized {
                add(prefix, sub)?;
            }
        }
        if let Some(sub) = custom {
            for prefix in ["en", "other", "ru", "uk", "ja"] {
                add(prefix, sub)?;
            }
        }
    }
    if previews.is_empty() {
        return Err(AssetError::Invalid("no valid keypad images"));
    }
    let mut resources = vec![Resource {
        relative_path: format!("{}/_big", target.directory()),
        data: vec![],
    }];
    resources.extend(items.into_iter().map(|(leaf, data)| Resource {
        relative_path: format!("{}/{leaf}", target.directory()),
        data,
    }));
    Ok(Theme {
        detected_versions: versions,
        target_version: target,
        resources,
        previews,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn png() -> Vec<u8> {
        encode_png(&image::DynamicImage::new_rgba8(8, 8)).unwrap()
    }
    fn zip(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }
    #[test]
    fn localized_bold_variants_and_previews_are_normalized() {
        let bytes = zip(&[("TelephonyUI-8/en-2-A B C--white.png", png())]);
        for (lang, expected) in [
            (Language::En, "en-2-A B C--white-bold.png"),
            (Language::Ru, "ru-2-А Б В Г--white-bold.png"),
            (Language::Uk, "uk-2-А Б В Г Ґ--white-bold.png"),
            (Language::Ja, "ja-2-ABC--white-bold.png"),
            (Language::All, "zh-2---white-bold.png"),
        ] {
            let theme = parse(&bytes, TelephonyVersion::V9, lang, true).unwrap();
            assert!(theme.detected_versions.contains("TelephonyUI-8"));
            assert!(
                theme
                    .resources
                    .iter()
                    .any(|r| r.relative_path == format!("TelephonyUI-9/{expected}"))
            );
            assert!(
                theme
                    .resources
                    .iter()
                    .any(|r| r.relative_path == "TelephonyUI-9/_big")
            );
            assert!(theme.previews["2"].starts_with(b"\x89PNG"));
        }
    }
    #[test]
    fn rejects_unsafe_ignored_entries_invalid_images_and_expansion() {
        for name in ["../.hidden", "/absolute", "a\\b", "a/../2.png"] {
            assert!(
                parse(
                    &zip(&[(name, vec![])]),
                    TelephonyVersion::V10,
                    Language::En,
                    false
                )
                .is_err()
            );
        }
        for data in [b"invalid PNG".to_vec(), vec![0; MAX_ENTRY_BYTES + 1]] {
            assert!(
                parse(
                    &zip(&[("2.png", data)]),
                    TelephonyVersion::V10,
                    Language::En,
                    false
                )
                .is_err()
            );
        }
        let mut symlink = zip::ZipWriter::new(Cursor::new(Vec::new()));
        symlink
            .add_symlink(
                "2.png",
                "elsewhere",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        assert!(
            parse(
                &symlink.finish().unwrap().into_inner(),
                TelephonyVersion::V10,
                Language::En,
                false
            )
            .is_err()
        );
    }
    #[test]
    fn rejects_ambiguous_key_images_and_trailing_data() {
        let second = encode_png(&image::DynamicImage::new_rgba8(9, 9)).unwrap();
        let bytes = zip(&[("en-2.png", png()), ("ru-2.png", second)]);
        assert!(parse(&bytes, TelephonyVersion::V10, Language::All, false).is_err());
        let mut bytes = zip(&[("2.png", png())]);
        bytes.push(0);
        assert!(parse(&bytes, TelephonyVersion::V10, Language::En, false).is_err());
    }
}
