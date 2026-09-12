use super::*;

#[tauri::command]
pub async fn gui_compatibility_rollback(
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<String, crate::command_error::CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let key = "gui.profile.tiktok-semantic-v1";
    let previous = state
        .db
        .get_setting(&format!("{key}.previous"))
        .map_err(crate::command_error::CommandError::operation)?
        .unwrap_or_default();
    state
        .db
        .set_setting(key, &previous)
        .map_err(crate::command_error::CommandError::operation)?;
    Ok("Đã khôi phục gói tương thích trước cho các phiên mới.".into())
}

#[tauri::command]
pub async fn gui_service_status(
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<GuiStatus, crate::command_error::CommandError> {
    state
        .gui_service
        .status()
        .await
        .map_err(crate::command_error::CommandError::operation)
}
#[tauri::command]
pub async fn gui_service_save(
    state: tauri::State<'_, crate::state::AppState>,
    config: GuiConfig,
) -> Result<(), crate::command_error::CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .gui_service
        .save(config)
        .await
        .map_err(crate::command_error::CommandError::operation)
}

#[tauri::command]
pub async fn gui_service_check(
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<String, crate::command_error::CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .gui_service
        .connection()
        .await
        .map_err(crate::command_error::CommandError::operation)?;
    Ok("Dịch vụ đã sẵn sàng, protocol v1 khớp. Model sẽ được kiểm chứng trên từng quan sát thực tế.".into())
}
#[tauri::command]
pub async fn gui_compatibility_import(
    state: tauri::State<'_, crate::state::AppState>,
    document: String,
) -> Result<String, crate::command_error::CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .gui_service
        .import_pack(document.as_bytes())
        .map(|hash| format!("Đã nhập gói tương thích {hash}. Phiên đang chạy giữ cấu hình cũ."))
        .map_err(crate::command_error::CommandError::operation)
}
#[tauri::command]
pub async fn gui_diagnostics_export(
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<String, crate::command_error::CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let service = state.gui_service.clone();
    tokio::task::spawn_blocking(move||->anyhow::Result<String>{
        use std::io::Write;
        std::fs::create_dir_all(&service.data_dir)?;
        let dest=service.data_dir.join(format!("diagnostics-{}.zip",uuid::Uuid::new_v4()));
        let mut archive=zip::ZipWriter::new(std::fs::File::create(&dest)?);
        let options=zip::write::SimpleFileOptions::default();
        archive.start_file("manifest.json",options)?;
        archive.write_all(serde_json::to_string_pretty(&serde_json::json!({"protocolVersion":1,"serviceVersion":"0.2.30","config":service.config()?} ))?.as_bytes())?;
        for name in ["trace.jsonl","trace.previous.jsonl"] {
            let trace=service.data_dir.join(name);
            if trace.is_file(){archive.start_file(name,options)?;std::io::copy(&mut std::fs::File::open(trace)?,&mut archive)?;}
        }
        let pictures=service.data_dir.join("observations");
        if pictures.is_dir() {
            for entry in std::fs::read_dir(pictures)?.take(128) {
                let path=entry?.path();
                if path.extension().is_some_and(|e|e=="png") && path.metadata()?.len()<=16*1024*1024 {
                    archive.start_file(format!("observations/{}",path.file_name().context("image name")?.to_string_lossy()),options)?;
                    std::io::copy(&mut std::fs::File::open(path)?,&mut archive)?;
                }
            }
        }
        archive.finish()?;
        Ok(format!("Đã xuất chẩn đoán: {}",dest.display()))
    }).await.map_err(crate::command_error::CommandError::operation)?.map_err(crate::command_error::CommandError::operation)
}
