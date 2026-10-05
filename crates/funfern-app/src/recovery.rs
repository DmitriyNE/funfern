use funfern_app::{
    topology_editor::TopologyDocument as Document, topology_persistence as persistence,
};

#[cfg(target_arch = "wasm32")]
const STORAGE_KEY: &str = "funfern.autosave.v1";
/// The guided tour's seen marker, beside the autosave: a launch with
/// nothing to restore opens the tour until it has been seen once.
#[cfg(target_arch = "wasm32")]
const GUIDE_KEY: &str = "funfern.guide.v1";

#[cfg(target_arch = "wasm32")]
pub fn guide_seen() -> bool {
    web_sys::window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(GUIDE_KEY).ok().flatten())
        .is_some()
}

#[cfg(target_arch = "wasm32")]
pub fn mark_guide_seen() -> Result<(), String> {
    let storage = web_sys::window()
        .ok_or("Browser window is unavailable")?
        .local_storage()
        .map_err(js_error)?
        .ok_or("Browser local storage is unavailable")?;
    storage.set_item(GUIDE_KEY, "seen").map_err(js_error)
}

#[cfg(target_arch = "wasm32")]
pub fn load() -> Result<Option<Vec<u8>>, String> {
    let storage = web_sys::window()
        .ok_or("Browser window is unavailable")?
        .local_storage()
        .map_err(js_error)?
        .ok_or("Browser local storage is unavailable")?;
    storage
        .get_item(STORAGE_KEY)
        .map_err(js_error)
        .map(|value| value.map(String::into_bytes))
}

#[cfg(target_arch = "wasm32")]
pub fn save(document: &Document) -> Result<(), String> {
    // Compact, since browser storage is a few megabytes an origin; the
    // reader takes either form.
    let json = String::from_utf8(persistence::save_compact(document)?)
        .map_err(|error| error.to_string())?;
    let storage = web_sys::window()
        .ok_or("Browser window is unavailable")?
        .local_storage()
        .map_err(js_error)?
        .ok_or("Browser local storage is unavailable")?;
    storage.set_item(STORAGE_KEY, &json).map_err(js_error)
}

#[cfg(target_arch = "wasm32")]
fn js_error(value: wasm_bindgen::JsValue) -> String {
    let field = |name: &str| {
        js_sys::Reflect::get(&value, &name.into())
            .ok()
            .and_then(|field| field.as_string())
    };
    if field("name").as_deref() == Some("QuotaExceededError") {
        return "Browser storage for this site is full".into();
    }
    value
        .as_string()
        .or_else(|| field("message"))
        .unwrap_or_else(|| "Browser storage operation failed".into())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load() -> Result<Option<Vec<u8>>, String> {
    let path = recovery_path()?;
    match std::fs::metadata(&path) {
        Ok(metadata) if metadata.len() > persistence::MAX_FILE_BYTES as u64 => Err(format!(
            "Autosave exceeds {} MiB",
            persistence::MAX_FILE_MIB
        )),
        Ok(_) => std::fs::read(path)
            .map(Some)
            .map_err(|error| format!("Could not read autosave: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Could not read autosave: {error}")),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save(document: &Document) -> Result<(), String> {
    let path = recovery_path()?;
    write_atomic(&path, persistence::save(document)?.as_bytes())
}

/// Whether the guided tour has been seen on this machine: a marker file
/// beside the autosave.
#[cfg(not(target_arch = "wasm32"))]
pub fn guide_seen() -> bool {
    guide_marker_path().is_ok_and(|path| path.exists())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn mark_guide_seen() -> Result<(), String> {
    let path = guide_marker_path()?;
    let parent = path.parent().ok_or("Marker path has no parent")?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create the marker's folder: {error}"))?;
    std::fs::write(&path, b"seen\n").map_err(|error| format!("Could not write the marker: {error}"))
}

#[cfg(not(target_arch = "wasm32"))]
fn guide_marker_path() -> Result<std::path::PathBuf, String> {
    Ok(recovery_path()?.with_file_name("guide-seen"))
}

#[cfg(not(target_arch = "wasm32"))]
fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("Autosave path has no parent")?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create autosave folder: {error}"))?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&temporary)
            .map_err(|error| format!("Could not create autosave: {error}"))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("Could not write autosave: {error}"))?;
    }
    std::fs::rename(&temporary, path).map_err(|error| format!("Could not commit autosave: {error}"))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn atomic_recovery_file_round_trips_the_complete_document() {
        let path =
            std::env::temp_dir().join(format!("funfern-autosave-test-{}.json", std::process::id()));
        let document = Document::default();
        let bytes = persistence::save(&document).unwrap();
        write_atomic(&path, bytes.as_bytes()).unwrap();
        let stored = std::fs::read(&path).unwrap();
        assert_eq!(persistence::parse_document(&stored).unwrap(), document);
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn recovery_path() -> Result<std::path::PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").ok_or("HOME is unavailable")?;
        Ok(
            std::path::PathBuf::from(home)
                .join("Library/Application Support/funfern/autosave.json"),
        )
    }
    #[cfg(target_os = "windows")]
    {
        let root = std::env::var_os("APPDATA").ok_or("APPDATA is unavailable")?;
        Ok(std::path::PathBuf::from(root).join("funfern/autosave.json"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        if let Some(root) = std::env::var_os("XDG_STATE_HOME") {
            return Ok(std::path::PathBuf::from(root).join("funfern/autosave.json"));
        }
        let home = std::env::var_os("HOME").ok_or("HOME is unavailable")?;
        Ok(std::path::PathBuf::from(home).join(".local/state/funfern/autosave.json"))
    }
}
