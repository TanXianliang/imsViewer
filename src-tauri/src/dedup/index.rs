use super::feature::{extract_features, ImageFeatures};
use super::model::SscdModel;
use super::source::{enumerate_sources, SourceItem};
use super::types::{
    DedupImageKind, DedupIndexConfig, DedupIndexStatus, DedupIndexSummary, DedupIndexUpdateSummary,
    EMBEDDING_DIM, MODEL_VERSION, PREPROCESS_VERSION,
};
use memmap2::{Mmap, MmapOptions};
use rayon::prelude::*;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const SCHEMA_VERSION: u32 = 1;
const VECTOR_BYTES: usize = EMBEDDING_DIM * std::mem::size_of::<f32>();

#[derive(Debug, Clone)]
pub struct IndexPaths {
    pub index_id: String,
    pub root_dir: PathBuf,
    pub database: PathBuf,
    pub vectors: PathBuf,
}

#[derive(Debug, Clone)]
pub struct IndexRecord {
    pub id: i64,
    pub absolute_path: String,
    pub relative_path: String,
    pub filename: String,
    pub kind: DedupImageKind,
    pub file_size: u64,
    pub modified_ns: i64,
    pub width: u32,
    pub height: u32,
    pub image_format: String,
    pub sha256: Vec<u8>,
    pub phash: u64,
    pub vector_offset: u64,
    pub vector_length: u64,
    pub feature_status: i64,
    pub error_message: Option<String>,
}

struct WorkItem {
    source: SourceItem,
    id: i64,
    existing: Option<IndexRecord>,
    reuse: bool,
}

struct PreparedItem {
    work: WorkItem,
    features: Option<Result<ImageFeatures, String>>,
}

pub fn index_id_for_root(root: &Path) -> String {
    let normalized = normalize_path(root);
    let digest = Sha256::digest(normalized.as_bytes());
    hex::encode(&digest[..12])
}

pub fn paths_for_root(app_data: &Path, root: &Path) -> IndexPaths {
    paths_for_id(app_data, &index_id_for_root(root))
}

pub fn paths_for_id(app_data: &Path, index_id: &str) -> IndexPaths {
    let root_dir = app_data.join("dedup").join(index_id);
    IndexPaths {
        index_id: index_id.to_string(),
        database: root_dir.join("index.sqlite3"),
        vectors: root_dir.join("vectors.bin"),
        root_dir,
    }
}

pub fn update_index<F>(
    app_data: &Path,
    config: &DedupIndexConfig,
    model: Arc<SscdModel>,
    cancel: &AtomicBool,
    progress: F,
) -> Result<DedupIndexUpdateSummary, String>
where
    F: Fn(u64, u64, Option<&Path>, &str) + Sync,
{
    build_index(app_data, config, model, cancel, false, progress)
}

pub fn rebuild_index<F>(
    app_data: &Path,
    config: &DedupIndexConfig,
    model: Arc<SscdModel>,
    cancel: &AtomicBool,
    progress: F,
) -> Result<DedupIndexUpdateSummary, String>
where
    F: Fn(u64, u64, Option<&Path>, &str) + Sync,
{
    build_index(app_data, config, model, cancel, true, progress)
}

fn build_index<F>(
    app_data: &Path,
    config: &DedupIndexConfig,
    model: Arc<SscdModel>,
    cancel: &AtomicBool,
    force_rebuild: bool,
    progress: F,
) -> Result<DedupIndexUpdateSummary, String>
where
    F: Fn(u64, u64, Option<&Path>, &str) + Sync,
{
    let started = Instant::now();
    let root = fs::canonicalize(&config.root_path)
        .map_err(|error| format!("图库路径无效 {}: {}", config.root_path, error))?;
    let paths = paths_for_root(app_data, &root);
    fs::create_dir_all(&paths.root_dir)
        .map_err(|error| format!("创建索引目录失败 {}: {}", paths.root_dir.display(), error))?;
    recover_index_files(&paths)?;

    progress(0, 0, None, "正在扫描图库");
    let sources = enumerate_sources(&root, config.recursive, config.max_depth)?;
    let total = sources.len() as u64;
    if cancel.load(Ordering::Relaxed) {
        return Err("索引更新已取消".to_string());
    }

    let (mut existing, old_vectors, old_model_version, old_preprocess_version) = if force_rebuild {
        (HashMap::new(), None, None, None)
    } else {
        load_existing(&paths)?
    };
    let can_reuse_model = old_model_version == Some(MODEL_VERSION)
        && old_preprocess_version == Some(PREPROCESS_VERSION);
    let mut next_id = existing.values().map(|record| record.id).max().unwrap_or(0) + 1;
    let mut work_items = Vec::with_capacity(sources.len());

    for source in sources {
        let key = normalize_path(&source.path);
        let old = existing.remove(&key);
        let id = old.as_ref().map(|record| record.id).unwrap_or_else(|| {
            let id = next_id;
            next_id += 1;
            id
        });
        let vector_valid = old.as_ref().map_or(false, |record| {
            if record.feature_status != 0 {
                return true;
            }
            let start = record.vector_offset as usize;
            let end = start.saturating_add(record.vector_length as usize);
            record.vector_length as usize == VECTOR_BYTES
                && old_vectors
                    .as_ref()
                    .map_or(false, |vectors| end <= vectors.len())
        });
        let reuse = old.as_ref().map_or(false, |record| {
            record.file_size == source.file_size
                && record.modified_ns == source.modified_ns
                && record.kind == source.kind
                && can_reuse_model
                && vector_valid
        });
        work_items.push(WorkItem {
            source,
            id,
            existing: old,
            reuse,
        });
    }

    let removed = existing.len() as u64;
    let database_tmp = paths.root_dir.join("index.sqlite3.tmp");
    let vectors_tmp = paths.root_dir.join("vectors.bin.tmp");
    remove_file_if_exists(&database_tmp)?;
    remove_file_if_exists(&vectors_tmp)?;
    let mut connection = create_database(&database_tmp)?;
    let mut vector_file =
        File::create(&vectors_tmp).map_err(|error| format!("创建向量临时文件失败: {}", error))?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("开始索引事务失败: {}", error))?;

    let worker_count = config
        .worker_count
        .unwrap_or_else(|| num_cpus::get().saturating_sub(1).clamp(1, 8))
        .max(1);
    let thread_pool = if config.multi_threaded {
        Some(
            rayon::ThreadPoolBuilder::new()
                .num_threads(worker_count)
                .build()
                .map_err(|error| format!("创建索引线程池失败: {}", error))?,
        )
    } else {
        None
    };

    let mut unchanged = 0u64;
    let mut added = 0u64;
    let mut updated = 0u64;
    let mut failed = 0u64;
    let mut completed = 0u64;
    let chunk_size = if config.multi_threaded {
        worker_count * 2
    } else {
        1
    };

    for chunk in work_items.chunks(chunk_size.max(1)) {
        if cancel.load(Ordering::Relaxed) {
            drop(transaction);
            drop(vector_file);
            drop(connection);
            remove_file_if_exists(&database_tmp)?;
            remove_file_if_exists(&vectors_tmp)?;
            return Err("索引更新已取消".to_string());
        }

        let prepare = || {
            chunk
                .into_par_iter()
                .map(|work| PreparedItem {
                    features: if work.reuse {
                        None
                    } else {
                        Some(extract_features(&work.source, &model))
                    },
                    work: WorkItem {
                        source: work.source.clone(),
                        id: work.id,
                        existing: work.existing.clone(),
                        reuse: work.reuse,
                    },
                })
                .collect::<Vec<_>>()
        };
        let prepared = if let Some(pool) = &thread_pool {
            pool.install(prepare)
        } else {
            chunk
                .iter()
                .map(|work| PreparedItem {
                    features: if work.reuse {
                        None
                    } else {
                        Some(extract_features(&work.source, &model))
                    },
                    work: WorkItem {
                        source: work.source.clone(),
                        id: work.id,
                        existing: work.existing.clone(),
                        reuse: work.reuse,
                    },
                })
                .collect()
        };

        for prepared_item in prepared {
            let existed_before = prepared_item.work.existing.is_some();
            let current_path = prepared_item.work.source.path.clone();
            let write_result = write_prepared_item(
                &transaction,
                &mut vector_file,
                old_vectors.as_ref(),
                prepared_item,
                &root,
            );
            match write_result {
                Ok(ItemWriteStatus::Reused) => unchanged += 1,
                Ok(ItemWriteStatus::Computed) if existed_before => updated += 1,
                Ok(ItemWriteStatus::Computed) => added += 1,
                Ok(ItemWriteStatus::Failed) => {
                    failed += 1;
                    if existed_before {
                        updated += 1;
                    } else {
                        added += 1;
                    }
                }
                Err(error) => {
                    drop(transaction);
                    drop(vector_file);
                    drop(connection);
                    remove_file_if_exists(&database_tmp)?;
                    remove_file_if_exists(&vectors_tmp)?;
                    return Err(error);
                }
            }
            completed += 1;
            progress(completed, total, Some(&current_path), "正在建立图片特征");
        }
    }

    let updated_ms = now_ms();
    write_meta(&transaction, "schema_version", &SCHEMA_VERSION.to_string())?;
    write_meta(&transaction, "root_path", &root.to_string_lossy())?;
    write_meta(&transaction, "model_version", &MODEL_VERSION.to_string())?;
    write_meta(
        &transaction,
        "preprocess_version",
        &PREPROCESS_VERSION.to_string(),
    )?;
    write_meta(&transaction, "last_updated_ms", &updated_ms.to_string())?;
    write_meta(&transaction, "item_count", &total.to_string())?;
    write_meta(&transaction, "state", "ready")?;
    transaction
        .commit()
        .map_err(|error| format!("提交索引事务失败: {}", error))?;
    vector_file
        .sync_all()
        .map_err(|error| format!("同步向量文件失败: {}", error))?;
    drop(vector_file);
    drop(connection);

    install_index_files(&paths, &database_tmp, &vectors_tmp)?;
    let total_bytes = directory_size(&paths.root_dir);
    Ok(DedupIndexUpdateSummary {
        index_id: paths.index_id,
        root_path: root.to_string_lossy().to_string(),
        unchanged,
        added,
        updated,
        removed,
        failed,
        final_count: total,
        total_bytes,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

enum ItemWriteStatus {
    Reused,
    Computed,
    Failed,
}

fn write_prepared_item(
    transaction: &rusqlite::Transaction<'_>,
    vector_file: &mut File,
    old_vectors: Option<&Mmap>,
    prepared: PreparedItem,
    root: &Path,
) -> Result<ItemWriteStatus, String> {
    let WorkItem {
        source,
        id,
        existing,
        reuse,
    } = prepared.work;
    let absolute_path = normalize_path(&source.path);
    let relative_path = source
        .path
        .strip_prefix(root)
        .unwrap_or(&source.path)
        .to_string_lossy()
        .to_string();
    let filename = source
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&relative_path)
        .to_string();

    if reuse {
        let old = existing.ok_or_else(|| "复用索引记录缺失".to_string())?;
        let (vector_offset, vector_length) = if old.feature_status == 0 {
            let vectors = old_vectors.ok_or_else(|| "旧向量文件缺失".to_string())?;
            let start = old.vector_offset as usize;
            let end = start + old.vector_length as usize;
            let offset = vector_file
                .stream_position()
                .map_err(|error| format!("获取向量偏移失败: {}", error))?;
            vector_file
                .write_all(&vectors[start..end])
                .map_err(|error| format!("复用向量失败: {}", error))?;
            (offset, old.vector_length)
        } else {
            (0, 0)
        };
        insert_record(
            transaction,
            id,
            &absolute_path,
            &relative_path,
            &filename,
            source.kind,
            source.file_size,
            source.modified_ns,
            old.width,
            old.height,
            &old.image_format,
            &old.sha256,
            old.phash,
            vector_offset,
            vector_length,
            old.feature_status,
            old.error_message.as_deref(),
        )?;
        return Ok(ItemWriteStatus::Reused);
    }

    match prepared
        .features
        .ok_or_else(|| "特征提取结果缺失".to_string())?
    {
        Ok(features) => {
            let vector_offset = vector_file
                .stream_position()
                .map_err(|error| format!("获取向量偏移失败: {}", error))?;
            for value in features.embedding {
                vector_file
                    .write_all(&value.to_le_bytes())
                    .map_err(|error| format!("写入向量失败: {}", error))?;
            }
            insert_record(
                transaction,
                id,
                &absolute_path,
                &relative_path,
                &filename,
                source.kind,
                source.file_size,
                source.modified_ns,
                features.width,
                features.height,
                &features.format_label,
                &features.sha256,
                features.phash,
                vector_offset,
                VECTOR_BYTES as u64,
                0,
                None,
            )?;
            Ok(ItemWriteStatus::Computed)
        }
        Err(error) => {
            insert_record(
                transaction,
                id,
                &absolute_path,
                &relative_path,
                &filename,
                source.kind,
                source.file_size,
                source.modified_ns,
                0,
                0,
                "未知",
                &[],
                0,
                0,
                0,
                1,
                Some(&error),
            )?;
            Ok(ItemWriteStatus::Failed)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn insert_record(
    transaction: &rusqlite::Transaction<'_>,
    id: i64,
    absolute_path: &str,
    relative_path: &str,
    filename: &str,
    kind: DedupImageKind,
    file_size: u64,
    modified_ns: i64,
    width: u32,
    height: u32,
    image_format: &str,
    sha256: &[u8],
    phash: u64,
    vector_offset: u64,
    vector_length: u64,
    feature_status: i64,
    error_message: Option<&str>,
) -> Result<(), String> {
    transaction
        .execute(
            "INSERT INTO image_items (
                id, absolute_path, relative_path, filename, kind, file_size, modified_ns,
                image_width, image_height, image_format, cover_sha256, phash,
                vector_offset, vector_length, feature_status, error_message
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                id,
                absolute_path,
                relative_path,
                filename,
                kind as i64,
                file_size as i64,
                modified_ns,
                width as i64,
                height as i64,
                image_format,
                sha256,
                phash as i64,
                vector_offset as i64,
                vector_length as i64,
                feature_status,
                error_message,
            ],
        )
        .map_err(|error| format!("写入索引记录失败 {}: {}", absolute_path, error))?;
    Ok(())
}

fn create_database(path: &Path) -> Result<Connection, String> {
    let connection =
        Connection::open(path).map_err(|error| format!("创建索引数据库失败: {}", error))?;
    connection
        .execute_batch(
            "PRAGMA journal_mode=OFF;
             PRAGMA synchronous=OFF;
             CREATE TABLE index_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE image_items (
                id INTEGER PRIMARY KEY,
                absolute_path TEXT NOT NULL UNIQUE,
                relative_path TEXT NOT NULL,
                filename TEXT NOT NULL,
                kind INTEGER NOT NULL,
                file_size INTEGER NOT NULL,
                modified_ns INTEGER NOT NULL,
                image_width INTEGER NOT NULL,
                image_height INTEGER NOT NULL,
                image_format TEXT NOT NULL,
                cover_sha256 BLOB NOT NULL,
                phash INTEGER NOT NULL,
                vector_offset INTEGER NOT NULL,
                vector_length INTEGER NOT NULL,
                feature_status INTEGER NOT NULL,
                error_message TEXT
             );
             CREATE INDEX idx_image_items_status ON image_items(feature_status);",
        )
        .map_err(|error| format!("初始化索引数据库失败: {}", error))?;
    Ok(connection)
}

fn load_existing(
    paths: &IndexPaths,
) -> Result<
    (
        HashMap<String, IndexRecord>,
        Option<Mmap>,
        Option<u32>,
        Option<u32>,
    ),
    String,
> {
    if !paths.database.is_file() {
        return Ok((HashMap::new(), None, None, None));
    }
    let connection = Connection::open(&paths.database)
        .map_err(|error| format!("打开旧索引数据库失败: {}", error))?;
    let model_version =
        read_meta(&connection, "model_version").and_then(|value| value.parse().ok());
    let preprocess_version =
        read_meta(&connection, "preprocess_version").and_then(|value| value.parse().ok());
    let records = read_records(&connection)?;
    let map = records
        .into_iter()
        .map(|record| (normalize_path(Path::new(&record.absolute_path)), record))
        .collect();
    let mmap = if paths.vectors.is_file() {
        let file =
            File::open(&paths.vectors).map_err(|error| format!("打开旧向量文件失败: {}", error))?;
        Some(
            unsafe { MmapOptions::new().map(&file) }
                .map_err(|error| format!("映射旧向量文件失败: {}", error))?,
        )
    } else {
        None
    };
    Ok((map, mmap, model_version, preprocess_version))
}

pub fn read_records(connection: &Connection) -> Result<Vec<IndexRecord>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, absolute_path, relative_path, filename, kind, file_size, modified_ns,
                    image_width, image_height, image_format, cover_sha256, phash,
                    vector_offset, vector_length, feature_status, error_message
             FROM image_items ORDER BY id",
        )
        .map_err(|error| format!("读取索引记录失败: {}", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok(IndexRecord {
                id: row.get(0)?,
                absolute_path: row.get(1)?,
                relative_path: row.get(2)?,
                filename: row.get(3)?,
                kind: DedupImageKind::from_i64(row.get(4)?),
                file_size: row.get::<_, i64>(5)? as u64,
                modified_ns: row.get(6)?,
                width: row.get::<_, i64>(7)? as u32,
                height: row.get::<_, i64>(8)? as u32,
                image_format: row.get(9)?,
                sha256: row.get(10)?,
                phash: row.get::<_, i64>(11)? as u64,
                vector_offset: row.get::<_, i64>(12)? as u64,
                vector_length: row.get::<_, i64>(13)? as u64,
                feature_status: row.get(14)?,
                error_message: row.get(15)?,
            })
        })
        .map_err(|error| format!("遍历索引记录失败: {}", error))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("解析索引记录失败: {}", error))
}

pub fn read_record_by_id(connection: &Connection, item_id: i64) -> Result<IndexRecord, String> {
    connection
        .query_row(
            "SELECT id, absolute_path, relative_path, filename, kind, file_size, modified_ns,
                    image_width, image_height, image_format, cover_sha256, phash,
                    vector_offset, vector_length, feature_status, error_message
             FROM image_items WHERE id=?1",
            [item_id],
            |row| {
                Ok(IndexRecord {
                    id: row.get(0)?,
                    absolute_path: row.get(1)?,
                    relative_path: row.get(2)?,
                    filename: row.get(3)?,
                    kind: DedupImageKind::from_i64(row.get(4)?),
                    file_size: row.get::<_, i64>(5)? as u64,
                    modified_ns: row.get(6)?,
                    width: row.get::<_, i64>(7)? as u32,
                    height: row.get::<_, i64>(8)? as u32,
                    image_format: row.get(9)?,
                    sha256: row.get(10)?,
                    phash: row.get::<_, i64>(11)? as u64,
                    vector_offset: row.get::<_, i64>(12)? as u64,
                    vector_length: row.get::<_, i64>(13)? as u64,
                    feature_status: row.get(14)?,
                    error_message: row.get(15)?,
                })
            },
        )
        .map_err(|error| format!("找不到索引结果 {}: {}", item_id, error))
}

pub fn get_index_status(app_data: &Path, root: &Path) -> Result<DedupIndexStatus, String> {
    let root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let paths = paths_for_root(app_data, &root);
    if !paths.database.is_file() {
        return Ok(DedupIndexStatus {
            index_id: paths.index_id,
            root_path: root.to_string_lossy().to_string(),
            exists: false,
            item_count: 0,
            total_bytes: 0,
            last_updated_ms: None,
            model_version: None,
            preprocess_version: None,
            state: "missing".to_string(),
        });
    }
    let connection = Connection::open(&paths.database)
        .map_err(|error| format!("打开索引数据库失败: {}", error))?;
    Ok(DedupIndexStatus {
        index_id: paths.index_id,
        root_path: read_meta(&connection, "root_path")
            .unwrap_or_else(|| root.to_string_lossy().to_string()),
        exists: true,
        item_count: read_meta(&connection, "item_count")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0),
        total_bytes: directory_size(&paths.root_dir),
        last_updated_ms: read_meta(&connection, "last_updated_ms")
            .and_then(|value| value.parse().ok()),
        model_version: read_meta(&connection, "model_version").and_then(|value| value.parse().ok()),
        preprocess_version: read_meta(&connection, "preprocess_version")
            .and_then(|value| value.parse().ok()),
        state: read_meta(&connection, "state").unwrap_or_else(|| "unknown".to_string()),
    })
}

pub fn list_indexes(app_data: &Path) -> Result<Vec<DedupIndexSummary>, String> {
    let base = app_data.join("dedup");
    if !base.is_dir() {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    for entry in fs::read_dir(&base).map_err(|error| format!("读取索引目录失败: {}", error))?
    {
        let entry = entry.map_err(|error| format!("读取索引条目失败: {}", error))?;
        if !entry.path().is_dir() {
            continue;
        }
        let index_id = entry.file_name().to_string_lossy().to_string();
        let paths = paths_for_id(app_data, &index_id);
        if !paths.database.is_file() {
            continue;
        }
        let connection = Connection::open(&paths.database)
            .map_err(|error| format!("打开索引数据库失败: {}", error))?;
        result.push(DedupIndexSummary {
            index_id,
            root_path: read_meta(&connection, "root_path").unwrap_or_default(),
            item_count: read_meta(&connection, "item_count")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
            total_bytes: directory_size(&paths.root_dir),
            last_updated_ms: read_meta(&connection, "last_updated_ms")
                .and_then(|value| value.parse().ok()),
            state: read_meta(&connection, "state").unwrap_or_else(|| "unknown".to_string()),
        });
    }
    result.sort_by(|left, right| right.last_updated_ms.cmp(&left.last_updated_ms));
    Ok(result)
}

pub fn delete_index(app_data: &Path, index_id: &str) -> Result<u64, String> {
    validate_index_id(index_id)?;
    let paths = paths_for_id(app_data, index_id);
    if !paths.root_dir.exists() {
        return Ok(0);
    }
    let bytes = directory_size(&paths.root_dir);
    let deleting = paths
        .root_dir
        .with_extension(format!("deleting-{}", uuid::Uuid::new_v4()));
    fs::rename(&paths.root_dir, &deleting)
        .map_err(|error| format!("准备删除索引失败: {}", error))?;
    fs::remove_dir_all(&deleting)
        .map_err(|error| format!("删除索引失败 {}: {}", deleting.display(), error))?;
    Ok(bytes)
}

pub fn delete_all_indexes(app_data: &Path) -> Result<(u64, u64), String> {
    let indexes = list_indexes(app_data)?;
    let mut count = 0u64;
    let mut bytes = 0u64;
    for index in indexes {
        bytes += delete_index(app_data, &index.index_id)?;
        count += 1;
    }
    Ok((count, bytes))
}

pub fn open_index(app_data: &Path, root: &Path) -> Result<(IndexPaths, Connection, Mmap), String> {
    let paths = paths_for_root(app_data, root);
    recover_index_files(&paths)?;
    if !paths.database.is_file() || !paths.vectors.is_file() {
        return Err(format!("图库尚未建立索引: {}", root.display()));
    }
    let connection = Connection::open(&paths.database)
        .map_err(|error| format!("打开索引数据库失败: {}", error))?;
    let file =
        File::open(&paths.vectors).map_err(|error| format!("打开向量文件失败: {}", error))?;
    let mmap = unsafe { MmapOptions::new().map(&file) }
        .map_err(|error| format!("映射向量文件失败: {}", error))?;
    Ok((paths, connection, mmap))
}

fn write_meta(
    transaction: &rusqlite::Transaction<'_>,
    key: &str,
    value: &str,
) -> Result<(), String> {
    transaction
        .execute(
            "INSERT OR REPLACE INTO index_meta(key, value) VALUES (?1, ?2)",
            params![key, value],
        )
        .map_err(|error| format!("写入索引元数据失败 {}: {}", key, error))?;
    Ok(())
}

fn read_meta(connection: &Connection, key: &str) -> Option<String> {
    connection
        .query_row("SELECT value FROM index_meta WHERE key=?1", [key], |row| {
            row.get(0)
        })
        .ok()
}

fn install_index_files(
    paths: &IndexPaths,
    database_tmp: &Path,
    vectors_tmp: &Path,
) -> Result<(), String> {
    let database_backup = paths.root_dir.join("index.sqlite3.bak");
    let vectors_backup = paths.root_dir.join("vectors.bin.bak");
    remove_file_if_exists(&database_backup)?;
    remove_file_if_exists(&vectors_backup)?;

    if paths.database.exists() {
        fs::rename(&paths.database, &database_backup)
            .map_err(|error| format!("备份旧数据库失败: {}", error))?;
    }
    if paths.vectors.exists() {
        fs::rename(&paths.vectors, &vectors_backup)
            .map_err(|error| format!("备份旧向量失败: {}", error))?;
    }

    let install_result = (|| {
        fs::rename(vectors_tmp, &paths.vectors)
            .map_err(|error| format!("安装新向量失败: {}", error))?;
        fs::rename(database_tmp, &paths.database)
            .map_err(|error| format!("安装新数据库失败: {}", error))?;
        Ok::<(), String>(())
    })();

    if let Err(error) = install_result {
        remove_file_if_exists(&paths.database)?;
        remove_file_if_exists(&paths.vectors)?;
        if database_backup.exists() {
            let _ = fs::rename(&database_backup, &paths.database);
        }
        if vectors_backup.exists() {
            let _ = fs::rename(&vectors_backup, &paths.vectors);
        }
        return Err(error);
    }

    remove_file_if_exists(&database_backup)?;
    remove_file_if_exists(&vectors_backup)?;
    Ok(())
}

fn recover_index_files(paths: &IndexPaths) -> Result<(), String> {
    let database_backup = paths.root_dir.join("index.sqlite3.bak");
    let vectors_backup = paths.root_dir.join("vectors.bin.bak");
    if !paths.database.exists() && database_backup.exists() {
        fs::rename(&database_backup, &paths.database)
            .map_err(|error| format!("恢复索引数据库失败: {}", error))?;
    }
    if !paths.vectors.exists() && vectors_backup.exists() {
        fs::rename(&vectors_backup, &paths.vectors)
            .map_err(|error| format!("恢复索引向量失败: {}", error))?;
    }
    remove_file_if_exists(&paths.root_dir.join("index.sqlite3.tmp"))?;
    remove_file_if_exists(&paths.root_dir.join("vectors.bin.tmp"))?;
    Ok(())
}

fn remove_file_if_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_file(path)
            .map_err(|error| format!("删除临时索引文件失败 {}: {}", path.display(), error))?;
    }
    Ok(())
}

fn validate_index_id(index_id: &str) -> Result<(), String> {
    if index_id.len() != 24
        || !index_id
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err("索引 ID 非法".to_string());
    }
    Ok(())
}

pub fn normalize_path(path: &Path) -> String {
    let normalized = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let value = normalized.to_string_lossy().replace('\\', "/");
    if cfg!(target_os = "windows") {
        value.trim_start_matches("//?/").to_lowercase()
    } else {
        value
    }
}

fn directory_size(path: &Path) -> u64 {
    WalkDirSize::new(path).sum()
}

struct WalkDirSize {
    stack: Vec<PathBuf>,
}

impl WalkDirSize {
    fn new(path: &Path) -> Self {
        Self {
            stack: vec![path.to_path_buf()],
        }
    }
}

impl Iterator for WalkDirSize {
    type Item = u64;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(path) = self.stack.pop() {
            if let Ok(metadata) = fs::metadata(&path) {
                if metadata.is_file() {
                    return Some(metadata.len());
                }
                if metadata.is_dir() {
                    if let Ok(entries) = fs::read_dir(path) {
                        self.stack
                            .extend(entries.flatten().map(|entry| entry.path()));
                    }
                }
            }
        }
        None
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_ids_are_stable() {
        let left = index_id_for_root(Path::new("C:/Images"));
        let right = index_id_for_root(Path::new("C:/Images"));
        assert_eq!(left, right);
        assert_eq!(left.len(), 24);
    }

    #[test]
    fn rejects_invalid_index_ids() {
        assert!(validate_index_id("../bad").is_err());
        assert!(validate_index_id("0123456789abcdef01234567").is_ok());
    }
}
