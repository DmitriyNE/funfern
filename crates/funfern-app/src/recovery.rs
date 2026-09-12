use funfern_app::{editor::Document, persistence};

#[cfg(target_arch = "wasm32")]
const STORAGE_KEY: &str = "funfern.autosave.v1";

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
    let json = persistence::save(document)?;
    let storage = web_sys::window()
        .ok_or("Browser window is unavailable")?
        .local_storage()
        .map_err(js_error)?
        .ok_or("Browser local storage is unavailable")?;
    storage.set_item(STORAGE_KEY, &json).map_err(js_error)
}

#[cfg(target_arch = "wasm32")]
fn js_error(value: wasm_bindgen::JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| "Browser storage operation failed".into())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load() -> Result<Option<Vec<u8>>, String> {
    let path = recovery_path()?;
    match std::fs::metadata(&path) {
        Ok(metadata) if metadata.len() > persistence::MAX_FILE_BYTES as u64 => {
            Err("Autosave exceeds 2 MiB".into())
        }
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
        let mut candidate = persistence::parse(&stored).unwrap();
        assert_eq!(candidate.advance(100_000).unwrap().unwrap(), document);
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
