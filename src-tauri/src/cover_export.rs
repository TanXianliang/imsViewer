use crate::ims::parse_ims_and_read_cover;
use crate::ims_pack::{scan_ims_files, ProgressPayload};
use crate::{output_image_filename, write_export_image};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::Emitter;

const PROGRESS_EVENT: &str = "batch-cover-export-progress";
static RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Deserialize)]
pub struct BatchCoverExportConfig {
    input_dir: String,
    output_dir: String,
    depth: u32,
    preserve_structure: bool,
    multi_threaded: bool,
}

struct RunGuard;

impl RunGuard {
    fn acquire() -> Result<Self, String> {
        RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| "已有封面导出任务正在运行，请等待其完成".into())
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::Release);
    }
}

#[derive(Debug)]
struct ExportTask {
    input_path: String,
    output_dir: PathBuf,
}

fn prepare_tasks(config: &BatchCoverExportConfig) -> Result<Vec<ExportTask>, String> {
    if config.input_dir.trim().is_empty() {
        return Err("请选择源目录".into());
    }
    let input_dir =
        std::fs::canonicalize(config.input_dir.trim()).map_err(|e| format!("源目录无效: {}", e))?;
    if !input_dir.is_dir() {
        return Err(format!("源路径不是目录: {}", input_dir.display()));
    }
    if config.output_dir.trim().is_empty() {
        return Err("请选择输出目录".into());
    }
    let output_root = PathBuf::from(config.output_dir.trim());
    let mut files = scan_ims_files(&input_dir.to_string_lossy(), config.depth.min(100));
    if files.is_empty() {
        return Err("未发现 IMS 文件".into());
    }
    // 顺序导出时保持同名封面的编号稳定。
    files.sort();
    let tasks = files
        .into_iter()
        .map(|input_path| {
            let output_dir = if config.preserve_structure {
                let parent = Path::new(&input_path).parent().unwrap_or(&input_dir);
                let relative = parent
                    .strip_prefix(&input_dir)
                    .map_err(|_| format!("文件不在源目录内: {}", input_path))?;
                output_root.join(relative)
            } else {
                output_root.clone()
            };
            Ok(ExportTask {
                input_path,
                output_dir,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    std::fs::create_dir_all(&output_root)
        .map_err(|e| format!("创建输出目录失败 {}: {}", output_root.display(), e))?;
    Ok(tasks)
}

fn export_cover(task: &ExportTask) -> Result<PathBuf, String> {
    // 只读 IMS 索引和封面，避免读取所有子图或重新编码封面。
    // 旧解析器对截断索引的部分字段使用 unwrap；单个坏文件应计入失败，不能中断整批。
    let (info, bytes) = std::panic::catch_unwind(|| parse_ims_and_read_cover(&task.input_path))
        .map_err(|_| "IMS 索引损坏，无法读取封面".to_string())??;
    let cover = info
        .images
        .iter()
        .rfind(|image| image.is_cover)
        .or_else(|| info.images.first())
        .ok_or_else(|| "IMS 文件中没有封面".to_string())?;
    if bytes.is_empty() {
        return Err("IMS 封面数据为空".into());
    }
    let filename = output_image_filename(&cover.filename, cover.format, cover.index);
    write_export_image(&task.output_dir.join(filename), &bytes)
}

#[tauri::command]
pub async fn start_batch_export_covers(
    app: tauri::AppHandle,
    config: BatchCoverExportConfig,
) -> Result<String, String> {
    let guard = RunGuard::acquire()?;
    tauri::async_runtime::spawn_blocking(move || {
        let tasks = prepare_tasks(&config)?;
        let total = tasks.len() as u32;
        let message = format!("共发现 {} 个 IMS 文件", total);
        let _ = app.emit(
            PROGRESS_EVENT,
            ProgressPayload {
                current: 0,
                total,
                message: message.clone(),
                status: "running".into(),
            },
        );
        std::thread::spawn(move || {
            let counts = Mutex::new((0u32, 0u32));
            let process = |task: &ExportTask| {
                let result = export_cover(task);
                // 计数和发送事件使用同一把锁，避免并行任务的进度倒退。
                let mut counts = counts.lock().unwrap();
                counts.0 += 1;
                let message = match result {
                    Ok(path) => format!("已导出 {}", path.display()),
                    Err(error) => {
                        counts.1 += 1;
                        format!("失败 {}: {}", task.input_path, error)
                    }
                };
                let _ = app.emit(
                    PROGRESS_EVENT,
                    ProgressPayload {
                        current: counts.0,
                        total,
                        message,
                        status: "running".into(),
                    },
                );
            };
            if config.multi_threaded {
                use rayon::prelude::*;
                tasks.par_iter().for_each(process);
            } else {
                tasks.iter().for_each(process);
            }
            let (_, failed) = *counts.lock().unwrap();
            // 先释放任务锁，前端收到完成事件后可以立即开始下一次导出。
            drop(guard);
            let _ = app.emit(
                PROGRESS_EVENT,
                ProgressPayload {
                    current: total,
                    total,
                    message: format!(
                        "封面导出完成，成功 {} 个，失败 {} 个",
                        total - failed,
                        failed
                    ),
                    status: if failed == 0 { "completed" } else { "error" }.into(),
                },
            );
        });
        Ok(message)
    })
    .await
    .map_err(|e| format!("启动封面导出失败: {}", e))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ims_pack::{save_ims_entries_atomic, UpEntry};
    use std::io::Cursor;
    use tempfile::tempdir;

    fn create_ims(path: &Path, cover_name: &str, color: u8) -> Vec<u8> {
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            4,
            3,
            image::Rgba([color, 20, 30, 128]),
        ));
        let mut cover = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut cover), image::ImageFormat::Png)
            .unwrap();
        let entries = vec![
            UpEntry {
                filename: cover_name.into(),
                format: 1,
                bytes: cover.clone(),
                is_cover: true,
            },
            UpEntry {
                filename: "sub.png".into(),
                format: 1,
                bytes: cover.clone(),
                is_cover: false,
            },
        ];
        save_ims_entries_atomic(&path.to_string_lossy(), &entries).unwrap();
        cover
    }

    fn config(input: &Path, output: &Path, preserve_structure: bool) -> BatchCoverExportConfig {
        BatchCoverExportConfig {
            input_dir: input.to_string_lossy().into(),
            output_dir: output.to_string_lossy().into(),
            depth: 0,
            preserve_structure,
            multi_threaded: false,
        }
    }

    #[test]
    fn preserves_relative_directories_and_exports_only_original_cover_bytes() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("源目录");
        let output = dir.path().join("输出目录");
        std::fs::create_dir_all(input.join("分类/子目录")).unwrap();
        let source = input.join("分类/子目录/a.ims.png");
        let cover = create_ims(&source, "a.png", 10);
        let original = std::fs::read(&source).unwrap();
        let root_cover = create_ims(&input.join("b.ims.png"), "b.png", 80);
        std::fs::write(input.join("普通图片.png"), &cover).unwrap();

        let tasks = prepare_tasks(&config(&input, &output, true)).unwrap();
        assert_eq!(tasks.len(), 2);
        for task in &tasks {
            export_cover(task).unwrap();
        }
        assert_eq!(
            std::fs::read(output.join("分类/子目录/a.png")).unwrap(),
            cover
        );
        assert_eq!(std::fs::read(output.join("b.png")).unwrap(), root_cover);
        assert!(!output.join("分类/子目录/sub.png").exists());
        assert!(!output.join("源目录").exists());
        assert!(!output.join("普通图片.png").exists());
        assert_eq!(std::fs::read(source).unwrap(), original);
    }

    #[test]
    fn flat_export_numbers_different_covers_and_reuses_identical_files() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("output");
        std::fs::create_dir_all(input.join("nested")).unwrap();
        let first = create_ims(&input.join("a.ims.png"), "cover.png", 10);
        let second = create_ims(&input.join("nested/b.ims.png"), "cover.png", 90);
        let tasks = prepare_tasks(&config(&input, &output, false)).unwrap();
        for task in &tasks {
            export_cover(task).unwrap();
            export_cover(task).unwrap();
        }
        assert_eq!(std::fs::read(output.join("cover.png")).unwrap(), first);
        assert_eq!(
            std::fs::read(output.join("cover_00001_.png")).unwrap(),
            second
        );
        assert_eq!(std::fs::read_dir(&output).unwrap().count(), 2);
    }

    #[test]
    fn respects_scan_depth_and_rejects_empty_or_invalid_directories() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("output");
        std::fs::create_dir_all(input.join("nested")).unwrap();
        let mut options = config(&input, &output, true);
        assert!(prepare_tasks(&options).unwrap_err().contains("未发现 IMS"));
        create_ims(&input.join("root.ims.png"), "root.png", 10);
        create_ims(&input.join("nested/deep.ims.png"), "deep.png", 20);
        options.depth = 1;
        assert_eq!(prepare_tasks(&options).unwrap().len(), 1);
        options.input_dir = input.join("root.ims.png").to_string_lossy().into();
        assert!(prepare_tasks(&options).unwrap_err().contains("不是目录"));
        options.input_dir = input.to_string_lossy().into();
        options.output_dir.clear();
        assert!(prepare_tasks(&options).unwrap_err().contains("输出目录"));
    }

    #[test]
    fn damaged_container_does_not_create_an_export() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("broken.ims.png");
        std::fs::write(&input, b"broken").unwrap();
        let output = dir.path().join("output");
        assert!(export_cover(&ExportTask {
            input_path: input.to_string_lossy().into(),
            output_dir: output.clone(),
        })
        .is_err());
        assert!(!output.exists());
        let mut truncated_index = b"IMSIMAGE".to_vec();
        truncated_index.extend_from_slice(&0u32.to_le_bytes());
        std::fs::write(&input, truncated_index).unwrap();
        assert!(export_cover(&ExportTask {
            input_path: input.to_string_lossy().into(),
            output_dir: output.clone(),
        })
        .unwrap_err()
        .contains("索引损坏"));
        assert!(!output.exists());
    }

    #[test]
    fn parallel_export_preserves_different_covers_with_the_same_name() {
        use rayon::prelude::*;

        let dir = tempdir().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("output");
        std::fs::create_dir_all(&input).unwrap();
        let first = create_ims(&input.join("a.ims.png"), "cover.png", 10);
        let second = create_ims(&input.join("b.ims.png"), "cover.png", 90);
        let tasks = prepare_tasks(&config(&input, &output, false)).unwrap();
        tasks.par_iter().for_each(|task| {
            export_cover(task).unwrap();
        });
        let actual = vec![
            std::fs::read(output.join("cover.png")).unwrap(),
            std::fs::read(output.join("cover_00001_.png")).unwrap(),
        ];
        assert!(actual.contains(&first));
        assert!(actual.contains(&second));
        assert_eq!(std::fs::read_dir(output).unwrap().count(), 2);
    }

    #[test]
    fn prevents_overlapping_runs_and_releases_the_guard() {
        let first = RunGuard::acquire().unwrap();
        assert!(RunGuard::acquire().is_err());
        drop(first);
        assert!(RunGuard::acquire().is_ok());
    }
}
