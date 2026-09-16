use funfern_app::topology_persistence::MAX_FILE_BYTES;
use std::sync::mpsc::Sender;
pub enum FileEvent {
    Loaded(Vec<u8>),
    SnapshotCaptured(Vec<u8>),
    Saved(&'static str),
    Cancelled,
    Error(String),
}

#[derive(Clone, Copy)]
pub enum SaveKind {
    Scene,
    SceneSvg,
    SnapshotPng,
}

impl SaveKind {
    fn file_name(self) -> &'static str {
        match self {
            Self::Scene => "funfern-scene.json",
            Self::SceneSvg => "funfern-scene.svg",
            Self::SnapshotPng => "funfern-snapshot.png",
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn filter(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::Scene => ("funfern scene", &["json"]),
            Self::SceneSvg => ("SVG image", &["svg"]),
            Self::SnapshotPng => ("PNG image", &["png"]),
        }
    }

    fn success(self) -> &'static str {
        match self {
            Self::Scene => "Scene saved",
            Self::SceneSvg => "Scene SVG exported",
            Self::SnapshotPng => "Snapshot exported",
        }
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub fn load(sender: Sender<FileEvent>) {
    std::thread::spawn(move || {
        let result = if let Some(path) = rfd::FileDialog::new()
            .add_filter("funfern scene", &["json"])
            .pick_file()
        {
            match std::fs::metadata(&path) {
                Ok(meta) if meta.len() <= MAX_FILE_BYTES as u64 => match std::fs::read(path) {
                    Ok(bytes) => FileEvent::Loaded(bytes),
                    Err(e) => FileEvent::Error(e.to_string()),
                },
                Ok(_) => FileEvent::Error("File exceeds 2 MiB".into()),
                Err(e) => FileEvent::Error(e.to_string()),
            }
        } else {
            FileEvent::Cancelled
        };
        let _ = sender.send(result);
    });
}
#[cfg(not(target_arch = "wasm32"))]
pub fn save(sender: Sender<FileEvent>, bytes: Vec<u8>, kind: SaveKind) {
    std::thread::spawn(move || {
        let (description, extensions) = kind.filter();
        let result = if let Some(path) = rfd::FileDialog::new()
            .add_filter(description, extensions)
            .set_file_name(kind.file_name())
            .save_file()
        {
            match std::fs::write(path, bytes) {
                Ok(()) => FileEvent::Saved(kind.success()),
                Err(e) => FileEvent::Error(e.to_string()),
            }
        } else {
            FileEvent::Cancelled
        };
        let _ = sender.send(result);
    });
}
#[cfg(target_arch = "wasm32")]
pub fn load(sender: Sender<FileEvent>) {
    wasm_bindgen_futures::spawn_local(async move {
        let result = if let Some(file) = rfd::AsyncFileDialog::new()
            .add_filter("funfern scene", &["json"])
            .pick_file()
            .await
        {
            if file.inner().size() > MAX_FILE_BYTES as f64 {
                FileEvent::Error("File exceeds 2 MiB".into())
            } else {
                FileEvent::Loaded(file.read().await)
            }
        } else {
            FileEvent::Cancelled
        };
        let _ = sender.send(result);
    });
}
#[cfg(target_arch = "wasm32")]
pub fn save(sender: Sender<FileEvent>, bytes: Vec<u8>, kind: SaveKind) {
    if matches!(kind, SaveKind::SnapshotPng) {
        let result = download(&bytes, kind.file_name())
            .map(|()| FileEvent::Saved(kind.success()))
            .unwrap_or_else(|error| {
                FileEvent::Error(format!("Could not download snapshot: {error:?}"))
            });
        let _ = sender.send(result);
        return;
    }
    wasm_bindgen_futures::spawn_local(async move {
        let result = if let Some(file) = rfd::AsyncFileDialog::new()
            .set_file_name(kind.file_name())
            .save_file()
            .await
        {
            match file.write(&bytes).await {
                Ok(()) => FileEvent::Saved(kind.success()),
                Err(e) => FileEvent::Error(e.to_string()),
            }
        } else {
            FileEvent::Cancelled
        };
        let _ = sender.send(result);
    });
}

#[cfg(target_arch = "wasm32")]
fn download(bytes: &[u8], file_name: &str) -> Result<(), wasm_bindgen::JsValue> {
    use wasm_bindgen::JsCast;

    let bytes = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::of1(&bytes.into());
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)?;
    let document = web_sys::window().unwrap().document().unwrap();
    let link = document
        .create_element("a")?
        .dyn_into::<web_sys::HtmlElement>()?;
    link.set_attribute("href", &url)?;
    link.set_attribute("download", file_name)?;
    link.click();
    web_sys::Url::revoke_object_url(&url)?;
    Ok(())
}
