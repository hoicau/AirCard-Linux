use crate::{Command, emit, local};
use aircard_core::{
    assets::{PreparedCard, Resource, resources_zip},
    passthm,
};
use device::{Error, ErrorKind, Result};
use serde_json::json;
fn resource_error(_: aircard_core::assets::AssetError) -> Error {
    Error::new(ErrorKind::InvalidInput, "resource_validation_or_limit")
}
fn summary(resources: &[Resource]) {
    emit(
        &json!({"event":"resource_plan","device_writes":false,"files":resources.iter().map(|r|json!({"path":r.relative_path,"bytes":r.data.len(),"sha256":aircard_core::sha256(&r.data)})).collect::<Vec<_>>()}),
    );
}
pub fn run(command: &Command) -> Result<Option<u8>> {
    match command {
        Command::PrepareCard {
            input,
            output,
            preview,
        } => {
            let card = PreparedCard::from_bytes(&local::read(
                input,
                aircard_core::assets::MAX_IMAGE_BYTES,
                false,
            )?)
            .map_err(resource_error)?;
            let resources = card.resources();
            let archive = output
                .as_ref()
                .map(|_| resources_zip(&resources))
                .transpose()
                .map_err(resource_error)?;
            // Roll back the first export if the second one fails. Existing files are never overwritten.
            if let Some(path) = preview {
                local::write_new(path, &card.png)?;
            }
            if let (Some(path), Some(bytes)) = (output, archive)
                && let Err(e) = local::write_new(path, &bytes)
            {
                if let Some(path) = preview {
                    local::remove(path)?;
                }
                return Err(e);
            }
            emit(
                &json!({"event":"card_prepared","source_size":card.source_size,"output_size":[1536,969],"preview_written":preview.is_some(),"archive_written":output.is_some()}),
            );
            summary(&resources);
        }
        Command::PrepareTheme {
            input,
            output,
            preview,
            language,
            telephony,
            bold,
        } => {
            let target = match telephony {
                8 => passthm::TelephonyVersion::V8,
                9 => passthm::TelephonyVersion::V9,
                _ => passthm::TelephonyVersion::V10,
            };
            let language = match language.as_str() {
                "ru" => passthm::Language::Ru,
                "uk" => passthm::Language::Uk,
                "ja" => passthm::Language::Ja,
                "all" => passthm::Language::All,
                _ => passthm::Language::En,
            };
            let theme = passthm::parse(
                &local::read(input, passthm::MAX_ARCHIVE_BYTES, false)?,
                target,
                language,
                *bold,
            )
            .map_err(resource_error)?;
            let archive = output
                .as_ref()
                .map(|_| resources_zip(&theme.resources))
                .transpose()
                .map_err(resource_error)?;
            let previews = theme
                .previews
                .iter()
                .map(|(key, data)| Resource {
                    relative_path: format!("key-{key}.png"),
                    data: data.clone(),
                })
                .collect::<Vec<_>>();
            if let Some(path) = preview {
                local::write_new(path, &resources_zip(&previews).map_err(resource_error)?)?;
            }
            if let (Some(path), Some(bytes)) = (output, archive)
                && let Err(e) = local::write_new(path, &bytes)
            {
                if let Some(path) = preview {
                    local::remove(path)?;
                }
                return Err(e);
            }
            emit(
                &json!({"event":"theme_prepared","detected_versions":theme.detected_versions,"target":theme.target_version.directory(),"keys":theme.previews.keys().collect::<Vec<_>>(),"preview_written":preview.is_some(),"archive_written":output.is_some()}),
            );
            summary(&theme.resources);
        }
        _ => return Ok(None),
    }
    Ok(Some(0))
}
