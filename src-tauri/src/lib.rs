//! IMS Viewer — Tauri Rust 后端
//!
//! 暴露给前端 (app.js) 的命令：
//!   open_file(path)    — 打开文件，扫描文件夹，解析 IMS，返回完整载荷
//!   pick_file()        — 弹出原生文件选择对话框
//!   get_sub_image()    — 获取当前 IMS 文件的指定子图数据
//!   extract_image()    — 导出单张图片
//!   extract_all()      — 导出全部子图

use base64::Engine;
use std::path::Path;
use tauri::Emitter;

mod ims;
mod ims_pack;

use ims::{scan_folder, parse_ims, read_sub_image, is_ims_file, file_type_label, FilePayload, SubImagePayload};
use ims_pack::{pack_ims, scan_multiple_folders, compute_output_path, upgrade_ims_file, scan_ims_files, BatchConfig, ProgressPayload};

// ============================================================
// Tauri Commands
// ============================================================

/// 打开一个图片文件（IMS 或普通图片）
///
/// 返回 FilePayload 包含：
/// - 文件夹中所有可浏览文件列表
/// - 当前文件的 IMS 元数据（如果是 IMS）
/// - 封面/首张图片的 base64 编码数据
#[tauri::command]
fn open_file(path: String) -> Result<FilePayload, String> {
    let filepath = Path::new(&path);

    if !filepath.exists() {
        return Err(format!("文件不存在: {}", path));
    }

    let filename = filepath
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&path)
        .to_string();

    let file_size = std::fs::metadata(&path)
        .map_err(|e| format!("读取文件信息失败: {}", e))?
        .len();

    let is_ims = is_ims_file(&path);

    // 扫描所在文件夹
    let parent_dir = filepath
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or(".");
    let folder_list = scan_folder(parent_dir)?;

    // 找到当前文件在文件夹列表中的位置
    let file_index = folder_list
        .iter()
        .position(|e| e.filepath == path)
        .unwrap_or(0);

    let file_type = file_type_label(&path);

    if is_ims {
        // ---- IMS 文件 ----
        let ims_info = parse_ims(&path)?;

        // 按需加载：仅封面或首张图在打开时编码，其余延迟到切换时加载
        let sub_images: Vec<SubImagePayload> = ims_info
            .images
            .iter()
            .map(|img| {
                let bytes_base64 = if img.is_cover || img.index == 0 {
                    let bytes = read_sub_image(&path, img.data_offset, img.data_length).unwrap_or_default();
                    base64::engine::general_purpose::STANDARD.encode(&bytes)
                } else {
                    String::new() // 延迟加载
                };
                SubImagePayload {
                    index: img.index,
                    filename: img.filename.clone(),
                    format: img.format,
                    data_offset: img.data_offset,
                    data_length: img.data_length,
                    is_cover: img.is_cover,
                    bytes_base64,
                    hash: img.hash.clone(),
                }
            })
            .collect();

        Ok(FilePayload {
            folder_list,
            file_index,
            filename,
            is_ims: true,
            file_size,
            file_type,
            ims_info: Some(ims_info),
            sub_images,
        })
    } else {
        // ---- 普通图片文件 ----
        let raw_bytes = std::fs::read(&path)
            .map_err(|e| format!("读取文件失败: {}", e))?;

        // 推断格式
        let format: u8 = if path.to_lowercase().ends_with(".png") { 1 } else { 0 };
        let bytes_base64 = base64::engine::general_purpose::STANDARD.encode(&raw_bytes);

        let sub_images = vec![SubImagePayload {
            index: 0,
            filename: filename.clone(),
            format,
            data_offset: 0,
            data_length: raw_bytes.len() as u32,
            is_cover: true,
            bytes_base64,
            hash: None,
        }];

        Ok(FilePayload {
            folder_list,
            file_index,
            filename,
            is_ims: false,
            file_size,
            file_type,
            ims_info: None,
            sub_images,
        })
    }
}

/// 弹出原生文件选择对话框，返回选中的文件路径
#[tauri::command]
async fn pick_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let result = app
        .dialog()
        .file()
        .add_filter("图片文件", &["jpg", "jpeg", "png", "ims.jpg", "ims.png"])
        .add_filter("IMS 文件", &["ims.jpg", "ims.png"])
        .add_filter("普通图片", &["jpg", "jpeg", "png"])
        .add_filter("所有文件", &["*"])
        .blocking_pick_file();

    Ok(result.map(|p| p.to_string()))
}

/// 弹出文件夹选择对话框
#[tauri::command]
async fn pick_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let result = app
        .dialog()
        .file()
        .blocking_pick_folder();

    Ok(result.map(|p| p.to_string()))
}

/// 按需获取 IMS 文件中指定索引的子图数据（base64）
#[tauri::command]
fn get_sub_image(path: String, index: usize) -> Result<SubImagePayload, String> {
    if !is_ims_file(&path) {
        return Err("非 IMS 文件，无法获取子图".into());
    }

    let ims_info = parse_ims(&path)?;

    let img = ims_info
        .images
        .get(index)
        .ok_or_else(|| format!("子图索引越界: {} (共 {} 张)", index, ims_info.image_count))?;

    let bytes = read_sub_image(&path, img.data_offset, img.data_length)?;
    let bytes_base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    Ok(SubImagePayload {
        index: img.index,
        filename: img.filename.clone(),
        format: img.format,
        data_offset: img.data_offset,
        data_length: img.data_length,
        is_cover: img.is_cover,
        bytes_base64,
        hash: img.hash.clone(),
    })
}

/// 导出单张图片——弹出保存对话框
#[tauri::command]
async fn extract_image(app: tauri::AppHandle, path: String, index: usize) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;

    let (bytes, default_name) = if is_ims_file(&path) {
        let ims_info = parse_ims(&path)?;
        let img = ims_info
            .images
            .get(index)
            .ok_or_else(|| "子图索引越界".to_string())?;
        let data = read_sub_image(&path, img.data_offset, img.data_length)?;

        // 使用原始文件名，保留扩展名
        let ext = if img.format == 1 { "png" } else { "jpg" };
        let stem = Path::new(&img.filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("image");
        (data, format!("{}.{}", stem, ext))
    } else {
        let data = std::fs::read(&path).map_err(|e| format!("读取文件失败: {}", e))?;
        let name = Path::new(&path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("export.jpg");
        (data, name.to_string())
    };

    // 弹出保存对话框
    let save_path = app
        .dialog()
        .file()
        .set_file_name(&default_name)
        .add_filter("JPEG 图片", &["jpg", "jpeg"])
        .add_filter("PNG 图片", &["png"])
        .blocking_save_file();

    if let Some(save_path) = save_path {
        let path_str = save_path.to_string();
        std::fs::write(Path::new(&path_str), &bytes)
            .map_err(|e| format!("写入文件失败: {}", e))?;
        Ok(path_str)
    } else {
        Err("用户取消了保存".into())
    }
}

/// 导出 IMS 文件中的全部子图到指定目录
#[tauri::command]
async fn extract_all(app: tauri::AppHandle, path: String) -> Result<Vec<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    if !is_ims_file(&path) {
        return Err("非 IMS 文件，无法导出子图".into());
    }

    // 选择输出目录
    let dir = app
        .dialog()
        .file()
        .blocking_pick_folder();

    if dir.is_none() {
        return Err("用户取消了选择".into());
    }

    let out_dir = dir.unwrap().to_string();
    let ims_info = parse_ims(&path)?;
    let mut saved_paths = Vec::new();

    for img in &ims_info.images {
        let bytes = read_sub_image(&path, img.data_offset, img.data_length)?;
        let ext = if img.format == 1 { "png" } else { "jpg" };
        let out_path = Path::new(&out_dir).join(format!("{}.{}",
            Path::new(&img.filename)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(&img.filename),
            ext
        ));
        std::fs::write(&out_path, &bytes)
            .map_err(|e| format!("写入失败 {}: {}", out_path.display(), e))?;
        saved_paths.push(out_path.to_string_lossy().to_string());
    }

    Ok(saved_paths)
}

/// 注册 imsViewer 到右键菜单"使用 imsViewer 打开"
#[tauri::command]
fn register_file_associations() -> Result<String, String> {
    let exe_path = std::env::current_exe()
        .map_err(|e| format!("获取 exe 路径失败: {}", e))?;
    let exe_str = exe_path.to_string_lossy().to_string();
    let exe_cmd = format!("\"{}\" \"%1\"", exe_str);
    let ico = format!("\"{}\",0", exe_str);

    let ext_groups: &[(&str, &[&str])] = &[
        ("image", &["jpg", "jpeg", "png", "webp", "avif"]),
        ("imscontainer", &["ims.jpg", "ims.png"]),
    ];

    for (_group, exts) in ext_groups {
        for ext in *exts {
            // 为每个扩展名添加右键菜单项
            let shell_path = format!(
                "HKCU\\Software\\Classes\\SystemFileAssociations\\.{}\\shell\\OpenWithImsViewer",
                ext
            );
            // 菜单显示名
            let _ = std::process::Command::new("reg")
                .args(["add", &shell_path, "/ve", "/d", "使用 imsViewer 打开", "/f"])
                .output();
            // 图标
            let _ = std::process::Command::new("reg")
                .args(["add", &shell_path, "/v", "Icon", "/d", &ico, "/f"])
                .output();
            // 命令
            let cmd_path = format!("{}\\command", shell_path);
            let _ = std::process::Command::new("reg")
                .args(["add", &cmd_path, "/ve", "/d", &exe_cmd, "/f"])
                .output();
        }
    }

    // 刷新 shell
    let _ = std::process::Command::new("ie4uinit").args(["-show"]).output();

    Ok("已添加到右键菜单".into())
}

/// 批量升级：扫描 IMS 文件，升级到最新版本（v2 + SHA-256 hash）
#[tauri::command]
async fn start_batch_upgrade(
    app: tauri::AppHandle,
    path: String,
    depth: u32,
    multi_threaded: bool,
) -> Result<String, String> {
    let files = scan_ims_files(&path, depth);
    let total = files.len() as u32;

    if total == 0 {
        return Err("未发现 IMS 文件".into());
    }

    let total_msg = format!("共发现 {} 个 IMS 文件", total);
    let _ = app.emit("batch-progress", ProgressPayload {
        current: 0,
        total,
        message: total_msg.clone(),
        status: "running".into(),
    });

    let app_handle = app.clone();

    std::thread::spawn(move || {
        let result = if multi_threaded {
            use rayon::prelude::*;
            use std::sync::atomic::{AtomicU32, AtomicBool, Ordering};

            let completed = AtomicU32::new(0);
            let has_error = AtomicBool::new(false);

            files.par_iter().for_each(|fp| {
                let fname = Path::new(fp).file_name().and_then(|n| n.to_str()).unwrap_or(fp);
                match upgrade_ims_file(fp) {
                    Ok(()) => {
                        let current = completed.fetch_add(1, Ordering::SeqCst) + 1;
                        let _ = app_handle.emit("batch-progress", ProgressPayload {
                            current,
                            total,
                            message: format!("已升级 {}", fname),
                            status: "running".into(),
                        });
                    }
                    Err(e) => {
                        has_error.store(true, Ordering::SeqCst);
                        let _ = app_handle.emit("batch-progress", ProgressPayload {
                            current: completed.load(Ordering::SeqCst),
                            total,
                            message: format!("失败 {}: {}", fname, e),
                            status: "running".into(),
                        });
                    }
                }
            });

            if has_error.load(Ordering::SeqCst) {
                Err("有文件升级失败".into())
            } else {
                Ok(())
            }
        } else {
            let mut completed: u32 = 0;
            for fp in &files {
                let fname = Path::new(fp).file_name().and_then(|n| n.to_str()).unwrap_or(fp);
                match upgrade_ims_file(fp) {
                    Ok(()) => {
                        completed += 1;
                        let _ = app_handle.emit("batch-progress", ProgressPayload {
                            current: completed,
                            total,
                            message: format!("已升级 {}", fname),
                            status: "running".into(),
                        });
                    }
                    Err(e) => {
                        let _ = app_handle.emit("batch-progress", ProgressPayload {
                            current: completed,
                            total,
                            message: format!("失败 {}: {}", fname, e),
                            status: "running".into(),
                        });
                    }
                }
            }
            Ok(())
        };

        match result {
            Ok(()) => {
                let _ = app_handle.emit("batch-progress", ProgressPayload {
                    current: total,
                    total,
                    message: format!("全部完成，共 {} 个文件", total),
                    status: "completed".into(),
                });
            }
            Err(e) => {
                let _ = app_handle.emit("batch-progress", ProgressPayload {
                    current: total,
                    total,
                    message: e,
                    status: "error".into(),
                });
            }
        }
    });

    Ok(total_msg)
}

/// 批量转换：扫描文件夹，按命名规范分组并打包为 IMS 文件
#[tauri::command]
async fn start_batch_convert(
    app: tauri::AppHandle,
    config: BatchConfig,
) -> Result<String, String> {
    // 1. 扫描所有文件夹，分组
    let groups = scan_multiple_folders(&config.folders)?;
    let total = groups.len() as u32;

    if total == 0 {
        return Err("没有找到可转换的图片组（需有封面 + 至少一张子图）".into());
    }

    let total_msg = format!("共发现 {} 组可转换", total);
    let _ = app.emit("batch-progress", ProgressPayload {
        current: 0,
        total,
        message: total_msg.clone(),
        status: "running".into(),
    });

    // 2. 构建任务列表
    let output_dir = config.output_dir.clone();
    struct Task {
        image_paths: Vec<String>,
        output_path: String,
        source_files: Vec<String>,
        message: String,
    }

    let tasks: Vec<Task> = groups.iter().map(|g| {
        let output_filename = format!("{}.ims.{}", g.base_name, g.cover_ext);
        let output_path = compute_output_path(g, &output_dir, config.preserve_structure);

        let mut image_paths = vec![g.cover.to_string_lossy().to_string()];
        let mut source_files = vec![g.cover.to_string_lossy().to_string()];
        for (_, sub_path) in &g.sub_images {
            image_paths.push(sub_path.to_string_lossy().to_string());
            source_files.push(sub_path.to_string_lossy().to_string());
        }

        Task {
            image_paths,
            output_path,
            source_files,
            message: format!("已转换 {}", output_filename),
        }
    }).collect();

    // 3. 执行转换
    let delete_source = config.delete_source;
    let multi = config.multi_threaded;
    let app_handle = app.clone();

    std::thread::spawn(move || {
        let convert = |task: &Task| -> Result<(), String> {
            pack_ims(&task.image_paths, 0, &task.output_path)?;
            if delete_source {
                for src in &task.source_files {
                    let _ = std::fs::remove_file(src);
                }
            }
            Ok(())
        };

        let result = if multi {
            use rayon::prelude::*;
            use std::sync::atomic::{AtomicU32, AtomicBool, Ordering};

            let completed = AtomicU32::new(0);
            let has_error = AtomicBool::new(false);

            tasks.par_iter().for_each(|task| {
                match convert(task) {
                    Ok(()) => {
                        let current = completed.fetch_add(1, Ordering::SeqCst) + 1;
                        let _ = app_handle.emit("batch-progress", ProgressPayload {
                            current,
                            total,
                            message: task.message.clone(),
                            status: "running".into(),
                        });
                    }
                    Err(e) => {
                        has_error.store(true, Ordering::SeqCst);
                        let _ = app_handle.emit("batch-progress", ProgressPayload {
                            current: completed.load(Ordering::SeqCst),
                            total,
                            message: format!("失败: {}", e),
                            status: "running".into(),
                        });
                    }
                }
            });

            if has_error.load(Ordering::SeqCst) {
                Err("有转换失败".into())
            } else {
                Ok(())
            }
        } else {
            let mut completed: u32 = 0;
            for task in &tasks {
                match convert(task) {
                    Ok(()) => {
                        completed += 1;
                        let _ = app_handle.emit("batch-progress", ProgressPayload {
                            current: completed,
                            total,
                            message: task.message.clone(),
                            status: "running".into(),
                        });
                    }
                    Err(e) => {
                        let _ = app_handle.emit("batch-progress", ProgressPayload {
                            current: completed,
                            total,
                            message: format!("失败: {}", e),
                            status: "running".into(),
                        });
                    }
                }
            }
            Ok(())
        };

        match result {
            Ok(()) => {
                let _ = app_handle.emit("batch-progress", ProgressPayload {
                    current: total,
                    total,
                    message: format!("全部完成，共 {} 组", total),
                    status: "completed".into(),
                });
            }
            Err(e) => {
                let _ = app_handle.emit("batch-progress", ProgressPayload {
                    current: total,
                    total,
                    message: e,
                    status: "error".into(),
                });
            }
        }
    });

    Ok(total_msg)
}

// ============================================================
// Tauri 应用入口
// ============================================================

/// 返回通过命令行传入的启动文件路径（右键"打开方式"传入）
#[tauri::command]
fn get_launch_file(state: tauri::State<'_, LaunchFile>) -> Result<Option<String>, String> {
    Ok(state.0.clone())
}

pub struct LaunchFile(pub Option<String>);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run(launch_file: Option<String>) {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(LaunchFile(launch_file))
        .invoke_handler(tauri::generate_handler![
            open_file,
            pick_file,
            pick_folder,
            get_sub_image,
            extract_image,
            extract_all,
            start_batch_convert,
            start_batch_upgrade,
            register_file_associations,
            get_launch_file,
        ])
        .run(tauri::generate_context!())
        .expect("启动 imsViewer 失败");
}
