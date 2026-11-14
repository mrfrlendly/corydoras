//! The optional workshop inventory. Switch the module on for each library.
use crate::{app_info_of, e2s, lib, AppInfo, AppState, CmdResult};
use corydoras_core::{
    ai::AiConfig,
    config::AppConfig,
    inventory::{InventoryDraft, InventoryItem, SupplierListing},
    inventory_label,
    inventory_media::InventoryPhotoChange,
    inventory_search, Library,
};
use std::sync::{atomic::Ordering, Arc};
use tauri::State;

fn current_library(state: &State<'_, AppState>) -> CmdResult<Arc<Library>> {
    if state.moving.load(Ordering::SeqCst) {
        return Err("Wait for the library move to finish.".into());
    }
    lib(state)
}

fn enabled_library(state: &State<'_, AppState>) -> CmdResult<Arc<Library>> {
    let library = current_library(state)?;
    if !library.inventory_enabled().map_err(e2s)? {
        return Err(
            "Workshop inventory is off for this library. Open Settings to turn it on.".into(),
        );
    }
    Ok(library)
}

#[tauri::command]
pub(super) fn set_inventory_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> CmdResult<AppInfo> {
    current_library(&state)?
        .set_inventory_enabled(enabled)
        .map_err(e2s)?;
    Ok(app_info_of(&state))
}

#[tauri::command]
pub(super) fn list_inventory(state: State<'_, AppState>) -> CmdResult<Vec<InventoryItem>> {
    enabled_library(&state)?.list_inventory().map_err(e2s)
}

#[tauri::command]
pub(super) fn save_inventory(
    state: State<'_, AppState>,
    id: Option<String>,
    draft: InventoryDraft,
    remove_photo: Option<bool>,
) -> CmdResult<InventoryItem> {
    let change = if remove_photo.unwrap_or(false) {
        InventoryPhotoChange::Remove
    } else {
        InventoryPhotoChange::Keep
    };
    enabled_library(&state)?
        .save_inventory_with_photo(id.as_deref(), &draft, change)
        .map_err(e2s)
}

#[derive(serde::Deserialize)]
struct PhotoForm {
    id: Option<String>,
    draft: InventoryDraft,
}

fn unpack_photo(bytes: &[u8]) -> CmdResult<(PhotoForm, &[u8])> {
    const MAX_FORM: usize = 128 * 1024;
    let header = bytes.get(..4).ok_or("Missing photo metadata.")?;
    let size = u32::from_be_bytes(header.try_into().expect("four bytes")) as usize;
    if size > MAX_FORM || size > bytes.len() - 4 {
        return Err("Invalid photo metadata size.".into());
    }
    let (metadata, photo) = bytes[4..].split_at(size);
    if photo.is_empty() || photo.len() > inventory_label::MAX_LABEL_BYTES {
        return Err("Select a nonempty photo of 12 MiB or less.".into());
    }
    let form = serde_json::from_slice(metadata).map_err(|_| "Invalid inventory fields.")?;
    Ok((form, photo))
}

#[tauri::command]
pub(super) async fn save_inventory_photo(
    state: State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> CmdResult<InventoryItem> {
    let library = enabled_library(&state)?;
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("Select an item photo.".into());
    };
    let (form, photo) = unpack_photo(bytes)?;
    static DECODER: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
    let slot = DECODER
        .try_acquire()
        .map_err(|_| "Wait for the current photo to finish.")?;
    let data = photo.to_vec();
    tauri::async_runtime::spawn_blocking(move || {
        let _slot = slot;
        library.save_inventory_with_photo(
            form.id.as_deref(),
            &form.draft,
            InventoryPhotoChange::Replace(&data),
        )
    })
    .await
    .map_err(e2s)?
    .map_err(e2s)
}

#[tauri::command]
pub(super) async fn get_inventory_photo(
    state: State<'_, AppState>,
    key: String,
    thumbnail: bool,
) -> CmdResult<String> {
    let library = enabled_library(&state)?;
    let bytes = library.inventory_photo(&key, thumbnail).map_err(e2s)?;
    Ok(corydoras_core::analysis::data_url_jpeg(&bytes))
}

#[tauri::command]
pub(super) fn delete_inventory(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    enabled_library(&state)?.delete_inventory(&id).map_err(e2s)
}

#[tauri::command]
pub(super) async fn read_inventory_label(
    state: State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> CmdResult<InventoryDraft> {
    let library = enabled_library(&state)?;
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("Select a label photo.".into());
    };
    if bytes.is_empty() || bytes.len() > inventory_label::MAX_LABEL_BYTES {
        return Err("Select a nonempty photo of 12 MiB or less.".into());
    }
    static READER: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
    let _slot = READER
        .try_acquire()
        .map_err(|_| "Wait for the current label to finish.")?;
    let settings = library.settings().map_err(e2s)?;
    let cfg = AiConfig::effective(AppConfig::load().ai.as_ref(), &settings);
    inventory_label::read_label(&library, &cfg, bytes)
        .await
        .map_err(e2s)
}

#[tauri::command]
pub(super) async fn search_inventory_listings(
    state: State<'_, AppState>,
    name: String,
    brand: String,
    part_number: String,
) -> CmdResult<Vec<SupplierListing>> {
    enabled_library(&state)?;
    inventory_search::search_listings(&name, &brand, &part_number)
        .await
        .map_err(e2s)
}
