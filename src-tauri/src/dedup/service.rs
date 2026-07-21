use super::index::{
    delete_all_indexes, delete_index, get_index_status, list_indexes, open_index,
    read_record_by_id, rebuild_index, update_index, usable_index_path,
};
use super::model::SscdModel;
use super::search::search_index;
use super::thumbnail::load_result_thumbnail;
use super::types::*;
use super::verify::{verify_pair, OcrVerifier};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
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
    index_lock: RwLock<()>,
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
                index_lock: RwLock::new(()),
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

    fn is_job_active(&self, job_id: &str) -> bool {
        self.inner
            .jobs
            .lock()
            .map(|jobs| jobs.contains_key(job_id))
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
                .write()
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
                .write()
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
                .read()
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
            .read()
            .map_err(|error| format!("锁定索引失败: {}", error))?;
        load_result_thumbnail(&app_data_dir(&app)?, Path::new(&root_path), item_id)
    })
    .await
    .map_err(|error| format!("缩略图后台任务失败: {}", error))?
}

#[tauri::command]
pub async fn dedup_delete_indexed_file(
    app: tauri::AppHandle,
    service: tauri::State<'_, DedupService>,
    root_path: String,
    item_id: i64,
) -> Result<DeleteDedupFileResult, String> {
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = service
            .inner
            .index_lock
            .write()
            .map_err(|error| format!("锁定索引失败: {}", error))?;
        let (_paths, connection, _vectors) =
            open_index(&app_data_dir(&app)?, Path::new(&root_path))?;
        let record = read_record_by_id(&connection, item_id)?;
        let filepath = usable_index_path(&record.absolute_path);
        if !filepath.is_file() {
            return Err(format!("图片文件不存在: {}", filepath.display()));
        }
        fs::remove_file(&filepath)
            .map_err(|error| format!("删除图片失败 {}: {}", filepath.display(), error))?;
        Ok(DeleteDedupFileResult {
            deleted: true,
            filepath: filepath.to_string_lossy().to_string(),
        })
    })
    .await
    .map_err(|error| format!("删除图片后台任务失败: {}", error))?
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
            .write()
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
            .write()
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

#[tauri::command]
pub async fn dedup_start_batch_scan(
    app: tauri::AppHandle,
    service: tauri::State<'_, DedupService>,
    job_id: String,
    request: BatchScanRequest,
) -> Result<BatchRunSummary, String> {
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let cancel = service.begin_job(&job_id)?;
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _guard = service
                .inner
                .index_lock
                .read()
                .map_err(|error| format!("锁定索引失败: {}", error))?;
            let app_data = app_data_dir(&app)?;
            let verify_service = service.clone();
            let verify_app = app.clone();
            let verify = move |left: &str,
                               right: &str,
                               similarity: f32|
                  -> Result<PairVerification, String> {
                let _guard = verify_service
                    .inner
                    .verify_lock
                    .lock()
                    .map_err(|error| format!("锁定复核任务失败: {}", error))?;
                let verifier = verify_service.verifier(&verify_app)?;
                let result = verify_pair(
                    &DedupVerificationRequest {
                        source_path: left.to_string(),
                        candidate_path: right.to_string(),
                        raw_similarity: similarity,
                    },
                    &verifier,
                )?;
                Ok(PairVerification {
                    local_verified: result.local_verified,
                    template_risk: result.template_risk,
                    reason: if result.local_verified {
                        "orb".to_string()
                    } else {
                        "ocr".to_string()
                    },
                })
            };
            let progress_app = app.clone();
            start_batch_scan(
                &app_data,
                &job_id,
                &request,
                &cancel,
                verify,
                move |payload| {
                    let _ = progress_app.emit("dedup-batch-progress", payload);
                },
            )
        }));
        service.finish_job(&job_id);
        match result {
            Ok(result) => result,
            Err(payload) => {
                let message = format!("批量查重内部错误：{}", panic_message(payload));
                if let Ok(app_data) = app_data_dir(&app) {
                    if let Ok(store) = BatchStore::open(&app_data, &job_id) {
                        let _ = store.set_status("failed", Some(&message), false);
                    }
                }
                Err(message)
            }
        }
    })
    .await
    .map_err(|error| format!("批量查重后台任务失败: {}", error))?
}

#[tauri::command]
pub async fn dedup_resume_batch_scan(
    app: tauri::AppHandle,
    service: tauri::State<'_, DedupService>,
    run_id: String,
) -> Result<BatchRunSummary, String> {
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let cancel = service.begin_job(&run_id)?;
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _guard = service
                .inner
                .index_lock
                .read()
                .map_err(|error| format!("锁定索引失败: {}", error))?;
            let app_data = app_data_dir(&app)?;
            let verify_service = service.clone();
            let verify_app = app.clone();
            let verify = move |left: &str,
                               right: &str,
                               similarity: f32|
                  -> Result<PairVerification, String> {
                let _guard = verify_service
                    .inner
                    .verify_lock
                    .lock()
                    .map_err(|error| format!("锁定复核任务失败: {}", error))?;
                let verifier = verify_service.verifier(&verify_app)?;
                let result = verify_pair(
                    &DedupVerificationRequest {
                        source_path: left.to_string(),
                        candidate_path: right.to_string(),
                        raw_similarity: similarity,
                    },
                    &verifier,
                )?;
                Ok(PairVerification {
                    local_verified: result.local_verified,
                    template_risk: result.template_risk,
                    reason: if result.local_verified {
                        "orb".to_string()
                    } else {
                        "ocr".to_string()
                    },
                })
            };
            let progress_app = app.clone();
            resume_batch_scan(&app_data, &run_id, &cancel, verify, move |payload| {
                let _ = progress_app.emit("dedup-batch-progress", payload);
            })
        }));
        service.finish_job(&run_id);
        match result {
            Ok(result) => result,
            Err(payload) => {
                let message = format!("继续批量查重内部错误：{}", panic_message(payload));
                if let Ok(app_data) = app_data_dir(&app) {
                    if let Ok(store) = BatchStore::open(&app_data, &run_id) {
                        let _ = store.set_status("failed", Some(&message), false);
                    }
                }
                Err(message)
            }
        }
    })
    .await
    .map_err(|error| format!("继续批量查重后台任务失败: {}", error))?
}

#[tauri::command]
pub fn dedup_list_batch_runs(
    app: tauri::AppHandle,
    service: tauri::State<'_, DedupService>,
    root_paths: Option<Vec<String>>,
) -> Result<Vec<BatchRunSummary>, String> {
    let app_data = app_data_dir(&app)?;
    let mut runs = BatchStore::list(&app_data)?;
    if let Some(filter) = root_paths {
        let filter = filter
            .iter()
            .map(|path| super::index::normalize_path(Path::new(path)))
            .collect::<std::collections::HashSet<_>>();
        runs.retain(|run| {
            run.root_paths
                .iter()
                .any(|path| filter.contains(&super::index::normalize_path(Path::new(path))))
        });
    }
    for run in &mut runs {
        let store = BatchStore::open(&app_data, &run.run_id)?;
        if matches!(run.status.as_str(), "preparing" | "running" | "finalizing")
            && !service.is_job_active(&run.run_id)
        {
            let resumable =
                run.mode == BatchScanMode::Complete && store.load_checkpoint()?.is_some();
            store.set_status("interrupted", Some("应用退出后任务已中断"), resumable)?;
            *run = store.summary()?;
        }
        match current_revision(&app_data, &run.root_paths) {
            Ok(revision) => {
                if store.mark_stale_if_needed(&revision)? {
                    *run = store.summary()?;
                }
            }
            Err(error) if !service.is_job_active(&run.run_id) => {
                store.set_status(
                    "stale",
                    Some(&format!("图库索引不存在或无法读取：{}", error)),
                    false,
                )?;
                *run = store.summary()?;
            }
            Err(_) => {}
        }
    }
    Ok(runs)
}

#[tauri::command]
pub fn dedup_get_batch_run(
    app: tauri::AppHandle,
    run_id: String,
) -> Result<BatchRunSummary, String> {
    BatchStore::open(&app_data_dir(&app)?, &run_id)?.summary()
}

#[tauri::command]
pub fn dedup_get_batch_groups(
    app: tauri::AppHandle,
    run_id: String,
    page: u64,
    page_size: u64,
) -> Result<BatchGroupPage, String> {
    BatchStore::open(&app_data_dir(&app)?, &run_id)?.get_groups(page, page_size)
}

#[tauri::command]
pub fn dedup_get_batch_group_members(
    app: tauri::AppHandle,
    run_id: String,
    group_id: i64,
    page: u64,
    page_size: u64,
) -> Result<BatchMemberPage, String> {
    BatchStore::open(&app_data_dir(&app)?, &run_id)?.get_group_members(group_id, page, page_size)
}

#[tauri::command]
pub fn dedup_preview_batch_auto_delete(
    app: tauri::AppHandle,
    service: tauri::State<'_, DedupService>,
    run_id: String,
    threshold: f32,
) -> Result<BatchAutoDeletePreview, String> {
    validate_auto_delete_threshold(threshold)?;
    if service.is_job_active(&run_id) {
        return Err("批量查重任务仍在运行，无法删除文件".to_string());
    }
    let app_data = app_data_dir(&app)?;
    let store = BatchStore::open(&app_data, &run_id)?;
    validate_auto_delete_run(&app_data, &store)?;
    let candidates = store.auto_delete_candidates(threshold)?;
    Ok(BatchAutoDeletePreview {
        threshold,
        candidate_count: candidates.len() as u64,
        candidate_bytes: candidates.iter().map(|member| member.file_size).sum(),
    })
}

#[tauri::command]
pub async fn dedup_auto_delete_batch_files(
    app: tauri::AppHandle,
    service: tauri::State<'_, DedupService>,
    run_id: String,
    threshold: f32,
) -> Result<BatchAutoDeleteResult, String> {
    validate_auto_delete_threshold(threshold)?;
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if service.is_job_active(&run_id) {
            return Err("批量查重任务仍在运行，无法删除文件".to_string());
        }
        let _guard = service
            .inner
            .index_lock
            .write()
            .map_err(|error| format!("锁定索引失败: {}", error))?;
        let app_data = app_data_dir(&app)?;
        let store = BatchStore::open(&app_data, &run_id)?;
        validate_auto_delete_run(&app_data, &store)?;
        let candidates = store.auto_delete_candidates(threshold)?;
        let candidate_count = candidates.len() as u64;
        let mut indexes = HashMap::new();
        let mut index_errors = HashMap::new();
        for root_path in candidates
            .iter()
            .map(|member| member.index_root_path.clone())
            .collect::<HashSet<_>>()
        {
            match open_index(&app_data, Path::new(&root_path)) {
                Ok((_paths, connection, vectors)) => {
                    indexes.insert(root_path, (connection, vectors));
                }
                Err(error) => {
                    index_errors.insert(root_path, error);
                }
            }
        }

        let mut deleted_count = 0_u64;
        let mut missing_count = 0_u64;
        let mut failed_count = 0_u64;
        let mut released_bytes = 0_u64;
        let mut removed_ids = HashSet::new();
        let mut failures = Vec::new();
        for member in candidates {
            let record = if let Some(error) = index_errors.get(&member.index_root_path) {
                failed_count += 1;
                push_auto_delete_failure(&mut failures, &member.filepath, error);
                continue;
            } else if let Some((connection, _vectors)) = indexes.get(&member.index_root_path) {
                match read_record_by_id(connection, member.item_id) {
                    Ok(record) => record,
                    Err(error) => {
                        failed_count += 1;
                        push_auto_delete_failure(&mut failures, &member.filepath, &error);
                        continue;
                    }
                }
            } else {
                failed_count += 1;
                push_auto_delete_failure(&mut failures, &member.filepath, "图库索引未打开");
                continue;
            };
            let filepath = usable_index_path(&record.absolute_path);
            if !filepath.is_file() {
                missing_count += 1;
                removed_ids.insert(member.batch_item_id);
                continue;
            }
            let file_size = fs::metadata(&filepath)
                .map(|metadata| metadata.len())
                .unwrap_or(member.file_size);
            match fs::remove_file(&filepath) {
                Ok(()) => {
                    deleted_count += 1;
                    released_bytes = released_bytes.saturating_add(file_size);
                    removed_ids.insert(member.batch_item_id);
                }
                Err(error) => {
                    failed_count += 1;
                    push_auto_delete_failure(
                        &mut failures,
                        &filepath.to_string_lossy(),
                        &error.to_string(),
                    );
                }
            }
        }
        let remaining_group_count = store.remove_members(&removed_ids)?;
        Ok(BatchAutoDeleteResult {
            threshold,
            candidate_count,
            deleted_count,
            missing_count,
            failed_count,
            released_bytes,
            remaining_group_count,
            failures,
        })
    })
    .await
    .map_err(|error| format!("一键删除后台任务失败: {}", error))?
}

#[tauri::command]
pub fn dedup_delete_batch_run(
    app: tauri::AppHandle,
    service: tauri::State<'_, DedupService>,
    run_id: String,
) -> Result<DeleteBatchRunResult, String> {
    if service.is_job_active(&run_id) {
        return Err("请先取消正在运行的批量任务".to_string());
    }
    let released_bytes = BatchStore::open(&app_data_dir(&app)?, &run_id)?.delete()?;
    Ok(DeleteBatchRunResult {
        deleted_runs: 1,
        released_bytes,
    })
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    "未知 panic".to_string()
}

fn validate_auto_delete_threshold(threshold: f32) -> Result<(), String> {
    if threshold.is_finite() && (0.0..=1.0).contains(&threshold) {
        Ok(())
    } else {
        Err("自动删除相似度必须在 0% 到 100% 之间".to_string())
    }
}

fn validate_auto_delete_run(app_data: &Path, store: &BatchStore) -> Result<(), String> {
    let summary = store.summary()?;
    if summary.status != "completed" {
        return Err("只能对已完成且未过期的批量查重结果执行一键删除".to_string());
    }
    let revision = current_revision(app_data, &summary.root_paths)?;
    store.validate_revision(&revision)
}

fn push_auto_delete_failure(
    failures: &mut Vec<BatchAutoDeleteFailure>,
    filepath: &str,
    error: &str,
) {
    if failures.len() < 100 {
        failures.push(BatchAutoDeleteFailure {
            filepath: filepath.to_string(),
            error: error.to_string(),
        });
    }
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
use super::batch::{current_revision, resume_batch_scan, start_batch_scan, PairVerification};
use super::batch_store::BatchStore;
