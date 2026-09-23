//! Offline card resources, adapted from AirCard-Windows v1.2.2 image_skin.rs (MIT).
//! No filesystem, GUI or device dependencies.
use image::{DynamicImage, GenericImageView, ImageFormat, ImageReader, imageops::FilterType};
use std::io::Cursor;
use thiserror::Error;

pub const CARD_WIDTH: u32 = 1536;
pub const CARD_HEIGHT: u32 = 969;
pub const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;
#[derive(Debug, Error)]
pub enum AssetError {
    #[error("image/archive exceeds the configured size, dimension or expansion limit")]
    Limit,
    #[error("invalid resource: {0}")]
    Invalid(&'static str),
    #[error("image decoding/encoding failed: {0}")]
    Image(#[from] image::ImageError),
    #[error("archive decoding/encoding failed: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("resource I/O failed: {0}")]
    Io(#[from] std::io::Error),
}
pub type Result<T> = std::result::Result<T, AssetError>;
#[derive(Clone)]
pub struct Resource {
    pub relative_path: String,
    pub data: Vec<u8>,
}
#[derive(Clone)]
pub struct PreparedCard {
    pub png: Vec<u8>,
    pub pdf: Vec<u8>,
    pub rgba: Vec<u8>,
    pub source_size: [u32; 2],
}
pub fn decode_image(bytes: &[u8]) -> Result<DynamicImage> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(AssetError::Limit);
    }
    let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    if !matches!(
        reader.format(),
        Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)
    ) {
        return Err(AssetError::Invalid("only PNG, JPEG and WebP are supported"));
    }
    let (w, h) = reader.into_dimensions()?;
    if w == 0 || h == 0 || w > 16384 || h > 16384 || u64::from(w) * u64::from(h) > 32 * 1024 * 1024
    {
        return Err(AssetError::Limit);
    }
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    Ok(reader.decode()?)
}
pub fn encode_png(image: &DynamicImage) -> Result<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    image.write_to(&mut output, ImageFormat::Png)?;
    let bytes = output.into_inner();
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(AssetError::Limit);
    }
    Ok(bytes)
}
impl PreparedCard {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Self::from_image(decode_image(bytes)?)
    }
    pub fn from_image(image: DynamicImage) -> Result<Self> {
        let (w, h) = image.dimensions();
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 32 * 1024 * 1024 {
            return Err(AssetError::Limit);
        }
        let crop = if u64::from(w) * u64::from(CARD_HEIGHT) > u64::from(h) * u64::from(CARD_WIDTH) {
            let width = ((u64::from(h) * u64::from(CARD_WIDTH) + u64::from(CARD_HEIGHT) / 2)
                / u64::from(CARD_HEIGHT))
            .clamp(1, u64::from(w)) as u32;
            image.crop_imm((w - width) / 2, 0, width, h)
        } else {
            let height = ((u64::from(w) * u64::from(CARD_HEIGHT) + u64::from(CARD_WIDTH) / 2)
                / u64::from(CARD_WIDTH))
            .clamp(1, u64::from(h)) as u32;
            image.crop_imm(0, (h - height) / 2, w, height)
        };
        let resized = crop.resize_exact(CARD_WIDTH, CARD_HEIGHT, FilterType::Lanczos3);
        let rgba = resized.to_rgba8().into_raw();
        let png = encode_png(&resized)?;
        let pdf = rgb_to_pdf(CARD_WIDTH, CARD_HEIGHT, &resized.to_rgb8().into_raw());
        Ok(Self {
            png,
            pdf,
            rgba,
            source_size: [w, h],
        })
    }
    pub fn resources(&self) -> Vec<Resource> {
        vec![
            Resource {
                relative_path: "Wallet/cardBackgroundCombined@3x.png".into(),
                data: self.png.clone(),
            },
            Resource {
                relative_path: "Wallet/cardBackgroundCombined@2x.png".into(),
                data: self.png.clone(),
            },
            Resource {
                relative_path: "Wallet/cardBackgroundCombined.pdf".into(),
                data: self.pdf.clone(),
            },
        ]
    }
}
/// Non-personal, unmistakable card artwork used for owner-approved hardware acceptance.
pub fn test_card() -> Result<PreparedCard> {
    let image = image::RgbaImage::from_fn(768, 484, |x, y| {
        let stripe = (x + y) / 48 % 2 == 0;
        image::Rgba([
            if stripe { 18 } else { 32 },
            (90 + y * 100 / 484) as u8,
            (170 + x * 70 / 768) as u8,
            255,
        ])
    });
    PreparedCard::from_image(DynamicImage::ImageRgba8(image))
}
fn rgb_to_pdf(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(rgb, 6);
    let content = format!("q\n{width} 0 0 {height} 0 0 cm\n/Im0 Do\nQ\n");
    let mut objects=vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Contents 4 0 R /Resources << /XObject << /Im0 5 0 R >> >> >>").into_bytes(),
        format!("<< /Length {} >>\nstream\n{content}endstream",content.len()).into_bytes(),
    ];
    let mut img=format!("<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",compressed.len()).into_bytes();
    img.extend(compressed);
    img.extend(b"\nendstream");
    objects.push(img);
    let mut pdf = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend(format!("{} 0 obj\n", i + 1).as_bytes());
        pdf.extend(object);
        pdf.extend(b"\nendobj\n");
    }
    let xref = pdf.len();
    pdf.extend(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        pdf.extend(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    pdf
}
/// Export resources in memory; every name is validated and total output is bounded.
pub fn resources_zip(resources: &[Resource]) -> Result<Vec<u8>> {
    use std::io::Write;
    if resources.len() > 2048 {
        return Err(AssetError::Limit);
    }
    let mut total = 0usize;
    let mut names = std::collections::BTreeSet::new();
    for item in resources {
        crate::safe_relative_path(&item.relative_path)
            .map_err(|_| AssetError::Invalid("unsafe export path"))?;
        if !names.insert(&item.relative_path) {
            return Err(AssetError::Invalid("duplicate export path"));
        }
        total = total
            .checked_add(item.data.len())
            .ok_or(AssetError::Limit)?;
        if total > 64 * 1024 * 1024 {
            return Err(AssetError::Limit);
        }
    }
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for item in resources {
        writer.start_file(
            &item.relative_path,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(0o600),
        )?;
        writer.write_all(&item.data)?;
    }
    Ok(writer.finish()?.into_inner())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn card_output_has_exact_size_and_preserves_center() {
        let mut input = image::RgbaImage::from_pixel(300, 100, image::Rgba([240, 0, 0, 255]));
        for x in 100..200 {
            for y in 0..100 {
                input.put_pixel(x, y, image::Rgba([0, 180, 80, 255]));
            }
        }
        let card = PreparedCard::from_image(DynamicImage::ImageRgba8(input)).unwrap();
        let output = decode_image(&card.png).unwrap();
        assert_eq!(output.dimensions(), (CARD_WIDTH, CARD_HEIGHT));
        assert_eq!(
            output
                .to_rgba8()
                .get_pixel(CARD_WIDTH / 2, CARD_HEIGHT / 2)
                .0,
            [0, 180, 80, 255]
        );
        assert!(card.pdf.starts_with(b"%PDF-1.4"));
        assert!(card.pdf.ends_with(b"%%EOF\n"));
        assert_eq!(card.resources().len(), 3);
    }
    #[test]
    fn archive_exports_reject_traversal_duplicates_and_oversize() {
        assert!(
            resources_zip(&[Resource {
                relative_path: "../escape".into(),
                data: vec![]
            }])
            .is_err()
        );
        let resource = Resource {
            relative_path: "safe.png".into(),
            data: vec![1],
        };
        assert!(resources_zip(&[resource.clone(), resource]).is_err());
        assert!(PreparedCard::from_bytes(b"not an image").is_err());
    }
}
