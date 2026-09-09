use funfern_app::persistence::MAX_FILE_BYTES;
use std::sync::mpsc::Sender;
pub enum FileEvent {
    Loaded(Vec<u8>),
    Saved,
    Cancelled,
    Error(String),
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
pub fn save(sender: Sender<FileEvent>, bytes: Vec<u8>) {
    std::thread::spawn(move || {
        let result = if let Some(path) = rfd::FileDialog::new()
            .add_filter("funfern scene", &["json"])
            .set_file_name("funfern-scene.json")
            .save_file()
        {
            match std::fs::write(path, bytes) {
                Ok(()) => FileEvent::Saved,
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
pub fn save(sender: Sender<FileEvent>, bytes: Vec<u8>) {
    wasm_bindgen_futures::spawn_local(async move {
        let result = if let Some(file) = rfd::AsyncFileDialog::new()
            .set_file_name("funfern-scene.json")
            .save_file()
            .await
        {
            match file.write(&bytes).await {
                Ok(()) => FileEvent::Saved,
                Err(e) => FileEvent::Error(e.to_string()),
            }
        } else {
            FileEvent::Cancelled
        };
        let _ = sender.send(result);
    });
}
