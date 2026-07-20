use super::index::{
    delete_all_indexes, delete_index, get_index_status, list_indexes, rebuild_index, update_index,
};
use super::model::SscdModel;
use super::search::search_index;
use super::thumbnail::load_result_thumbnail;
use super::types::*;
use super::verify::{verify_pair, OcrVerifier};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::path::BaseDirectory;
use tauri::{Emitter, Manager};

#[derive(Clone)]
pub struct DedupService {
    inner: Arc<DedupServiceInner>,
}

struct DedupServiceInner {
    model: Mutex<Option<Arc<SscdModel>>>,
    verifier: Mutex<Option<Arc<OcrVerifier>>>,
    jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    index_lock: Mutex<()>,
    thumbnail_lock: Mutex<()>,
    verify_lock: Mutex<()>,
}

#[derive(Deserialize)]
struct ModelMetadata {
    onnx_sha256: String,
}

impl DedupService {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DedupServiceInner {
                model: Mutex::new(None),
                verifier: Mutex::new(None),
                jobs: Mutex::new(HashMap::new()),
                index_lock: Mutex::new(()),
                thumbnail_lock: Mutex::new(()),
                verify_lock: Mutex::new(()),
            }),
        }
    }

    fn verifier(&self, app: &tauri::AppHandle) -> Result<Arc<OcrVerifier>, String> {
        let mut slot = self
            .inner
            .verifier
            .lock()
            .map_err(|error| format!("锁定 OCR 状态失败: {}", error))?;
        if let Some(verifier) = slot.as_ref() {
            return Ok(verifier.clone());
        }
        let detection_path = resolve_model_resource(app, "text-detection.rten")?;
        let recognition_path = resolve_model_resource(app, "text-recognition.rten")?;
        let verifier = Arc::new(OcrVerifier::load(&detection_path, &recognition_path)?);
        *slot = Some(verifier.clone());
        Ok(verifier)
    }

    fn model(&self, app: &tauri::AppHandle) -> Result<Arc<SscdModel>, String> {
        let mut slot = self
            .inner
            .model
            .lock()
            .map_err(|error| format!("锁定模型状态失败: {}", error))?;
        if let Some(model) = slot.as_ref() {
            return Ok(model.clone());
        }
        let model_path = resolve_model_path(app)?;
        let metadata_path = model_path.with_file_name("sscd_disc_mixup.json");
        let metadata: ModelMetadata =
            serde_json::from_slice(&fs::read(&metadata_path).map_err(|error| {
                format!("读取模型配置失败 {}: {}", metadata_path.display(), error)
            })?)
            .map_err(|error| format!("解析模型配置失败: {}", error))?;
        let model = Arc::new(SscdModel::load(
            &model_path,
            true,
            num_cpus::get().saturating_sub(1).clamp(1, 8),
        )?);
        if !model
            .model_sha256()
            .eq_ignore_ascii_case(&metadata.onnx_sha256)
        {
            return Err("SSCD 模型校验失败，请重新安装 imsViewer".to_string());
        }
        *slot = Some(model.clone());
        Ok(model)
    }

    fn begin_job(&self, job_id: &str) -> Result<Arc<AtomicBool>, String> {
        let mut jobs = self
            .inner
            .jobs
            .lock()
            .map_err(|error| format!("锁定任务状态失败: {}", error))?;
        if jobs.contains_key(job_id) {
            return Err(format!("任务 ID 已存在: {}", job_id));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        jobs.insert(job_id.to_string(), cancel.clone());
        Ok(cancel)
    }

    fn finish_job(&self, job_id: &str) {
        if let Ok(mut jobs) = self.inner.jobs.lock() {
            jobs.remove(job_id);
        }
    }

    pub fn cancel_job(&self, job_id: &str) -> bool {
        self.inner
            .jobs
            .lock()
            .ok()
            .and_then(|jobs| jobs.get(job_id).cloned())
            .map(|cancel| {
                cancel.store(true, Ordering::Relaxed);
                true
            })
            .unwrap_or(false)
    }
}

#[tauri::command]
pub fn dedup_get_model_status(
    app: tauri::AppHandle,
    service: tauri::State<'_, DedupService>,
) -> DedupModelStatus {
    match service.model(&app) {
        Ok(model) => DedupModelStatus {
            available: true,
            model_id: MODEL_ID.to_string(),
            model_version: MODEL_VERSION,
            provider: Some(model.provider().to_string()),
            model_path: Some(model.model_path().to_string_lossy().to_string()),
            model_sha256: Some(model.model_sha256().to_string()),
            error: None,
        },
        Err(error) => DedupModelStatus {
            available: false,
            model_id: MODEL_ID.to_string(),
            model_version: MODEL_VERSION,
            provider: None,
            model_path: None,
            model_sha256: None,
            error: Some(error),
        },
    }
}

#[tauri::command]
pub fn dedup_get_index_status(
    app: tauri::AppHandle,
    root_path: String,
) -> Result<DedupIndexStatus, String> {
    get_index_status(&app_data_dir(&app)?, Path::new(&root_path))
}

#[tauri::command]
pub fn dedup_list_indexes(app: tauri::AppHandle) -> Result<Vec<DedupIndexSummary>, String> {
    list_indexes(&app_data_dir(&app)?)
}

#[tauri::command]
pub async fn dedup_update_index(
    app: tauri::AppHandle,
    service: tauri::State<'_, DedupService>,
    job_id: String,
    config: DedupIndexConfig,
) -> Result<DedupIndexUpdateSummary, String> {
    let service = service.inner().clone();
    let app_for_job = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let cancel = service.begin_job(&job_id)?;
        let result = (|| {
            let _guard = service
                .inner
                .index_lock
                .lock()
                .map_err(|error| format!("锁定索引失败: {}", error))?;
            let model = service.model(&app_for_job)?;
            let app_data = app_data_dir(&app_for_job)?;
            let progress_app = app_for_job.clone();
            let progress_job = job_id.clone();
            update_index(
                &app_data,
                &config,
                model,
                &cancel,
                move |current, total, path, message| {
                    let _ = progress_app.emit(
                        "dedup-index-progress",
                        DedupProgressPayload {
                            job_id: progress_job.clone(),
                            phase: "index".to_string(),
                            current,
                            total,
                            current_file: path.map(|value| value.to_string_lossy().to_string()),
                            message: message.to_string(),
                            status: "running".to_string(),
                        },
                    );
                },
            )
        })();
        service.finish_job(&job_id);
        result
    })
    .await
    .map_err(|error| format!("索引后台任务失败: {}", error))?
}

#[tauri::command]
pub async fn dedup_rebuild_index(
    app: tauri::AppHandle,
    service: tauri::State<'_, DedupService>,
    job_id: String,
    config: DedupIndexConfig,
) -> Result<DedupIndexUpdateSummary, String> {
    let service = service.inner().clone();
    let app_for_job = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let cancel = service.begin_job(&job_id)?;
        let result = (|| {
            let _guard = service
                .inner
                .index_lock
                .lock()
                .map_err(|error| format!("锁定索引失败: {}", error))?;
            let model = service.model(&app_for_job)?;
            let app_data = app_data_dir(&app_for_job)?;
            let progress_app = app_for_job.clone();
            let progress_job = job_id.clone();
            rebuild_index(
                &app_data,
                &config,
                model,
                &cancel,
                move |current, total, path, message| {
                    let _ = progress_app.emit(
                        "dedup-index-progress",
                        DedupProgressPayload {
                            job_id: progress_job.clone(),
                            phase: "rebuild".to_string(),
                            current,
                            total,
                            current_file: path.map(|value| value.to_string_lossy().to_string()),
                            message: message.to_string(),
                            status: "running".to_string(),
                        },
                    );
                },
            )
        })();
        service.finish_job(&job_id);
        result
    })
    .await
    .map_err(|error| format!("索引重建后台任务失败: {}", error))?
}

#[tauri::command]
pub async fn dedup_search(
    app: tauri::AppHandle,
    service: tauri::State<'_, DedupService>,
    job_id: String,
    request: DedupSearchRequest,
) -> Result<DedupSearchResponse, String> {
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let cancel = service.begin_job(&job_id)?;
        let result = (|| {
            let _guard = service
                .inner
                .index_lock
                .lock()
                .map_err(|error| format!("锁定索引失败: {}", error))?;
            let model = service.model(&app)?;
            search_index(&app_data_dir(&app)?, &request, model, &cancel)
        })();
        service.finish_job(&job_id);
        result
    })
    .await
    .map_err(|error| format!("查重后台任务失败: {}", error))?
}

#[tauri::command]
pub fn dedup_cancel_job(service: tauri::State<'_, DedupService>, job_id: String) -> bool {
    service.cancel_job(&job_id)
}

#[tauri::command]
pub async fn dedup_get_result_thumbnail(
    app: tauri::AppHandle,
    service: tauri::State<'_, DedupService>,
    root_path: String,
    item_id: i64,
) -> Result<ThumbnailPayload, String> {
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _thumbnail_guard = service
            .inner
            .thumbnail_lock
            .lock()
            .map_err(|error| format!("锁定缩略图队列失败: {}", error))?;
        let _index_guard = service
            .inner
            .index_lock
            .lock()
            .map_err(|error| format!("锁定索引失败: {}", error))?;
        load_result_thumbnail(&app_data_dir(&app)?, Path::new(&root_path), item_id)
    })
    .await
    .map_err(|error| format!("缩略图后台任务失败: {}", error))?
}

#[tauri::command]
pub async fn dedup_verify_result(
    app: tauri::AppHandle,
    service: tauri::State<'_, DedupService>,
    request: DedupVerificationRequest,
) -> Result<DedupVerificationResult, String> {
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = service
            .inner
            .verify_lock
            .lock()
            .map_err(|error| format!("锁定复核任务失败: {}", error))?;
        let verifier = service.verifier(&app)?;
        verify_pair(&request, &verifier)
    })
    .await
    .map_err(|error| format!("图片复核后台任务失败: {}", error))?
}

#[tauri::command]
pub async fn dedup_delete_index(
    app: tauri::AppHandle,
    service: tauri::State<'_, DedupService>,
    index_id: String,
) -> Result<DeleteDedupIndexResult, String> {
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = service
            .inner
            .index_lock
            .lock()
            .map_err(|error| format!("锁定索引失败: {}", error))?;
        let released_bytes = delete_index(&app_data_dir(&app)?, &index_id)?;
        Ok(DeleteDedupIndexResult {
            deleted_indexes: u64::from(released_bytes > 0),
            released_bytes,
        })
    })
    .await
    .map_err(|error| format!("删除索引后台任务失败: {}", error))?
}

#[tauri::command]
pub async fn dedup_delete_all_indexes(
    app: tauri::AppHandle,
    service: tauri::State<'_, DedupService>,
) -> Result<DeleteDedupIndexResult, String> {
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = service
            .inner
            .index_lock
            .lock()
            .map_err(|error| format!("锁定索引失败: {}", error))?;
        let (deleted_indexes, released_bytes) = delete_all_indexes(&app_data_dir(&app)?)?;
        Ok(DeleteDedupIndexResult {
            deleted_indexes,
            released_bytes,
        })
    })
    .await
    .map_err(|error| format!("删除全部索引后台任务失败: {}", error))?
}

#[tauri::command]
pub fn dedup_reveal_file(path: String) -> Result<(), String> {
    if !Path::new(&path).exists() {
        return Err(format!("文件不存在: {}", path));
    }
    Command::new("explorer.exe")
        .arg(format!("/select,{}", path))
        .spawn()
        .map_err(|error| format!("打开资源管理器失败: {}", error))?;
    Ok(())
}

fn app_data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let path = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("获取应用数据目录失败: {}", error))?;
    fs::create_dir_all(&path).map_err(|error| format!("创建应用数据目录失败: {}", error))?;
    Ok(path)
}

fn resolve_model_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    resolve_model_resource(app, "sscd_disc_mixup.onnx")
}

fn resolve_model_resource(app: &tauri::AppHandle, filename: &str) -> Result<PathBuf, String> {
    let relative = format!("resources/models/{}", filename);
    let resource_path = app
        .path()
        .resolve(&relative, BaseDirectory::Resource)
        .map_err(|error| format!("定位模型资源失败 {}: {}", filename, error))?;
    if resource_path.is_file() {
        return Ok(resource_path);
    }
    let development_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("models")
        .join(filename);
    if development_path.is_file() {
        return Ok(development_path);
    }
    Err(format!("未找到模型资源: {}", filename))
}
