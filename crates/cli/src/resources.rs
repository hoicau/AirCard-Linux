use crate::{Command, emit, local};
use aircard_core::assets::{PreparedCard, Resource, resources_zip};
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
        _ => return Ok(None),
    }
    Ok(Some(0))
}
