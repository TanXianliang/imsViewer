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

mod ims;

use ims::{scan_folder, parse_ims, read_sub_image, is_ims_file, file_type_label, FilePayload, SubImagePayload};

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

        // 构建 sub_images：封面数据 + 其余仅元数据
        let sub_images: Vec<SubImagePayload> = ims_info
            .images
            .iter()
            .map(|img| {
                let bytes = read_sub_image(&path, img.data_offset, img.data_length).unwrap_or_default();
                let bytes_base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                SubImagePayload {
                    index: img.index,
                    filename: img.filename.clone(),
                    format: img.format,
                    data_offset: img.data_offset,
                    data_length: img.data_length,
                    is_cover: img.is_cover,
                    bytes_base64,
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

// ============================================================
// Tauri 应用入口
// ============================================================

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            open_file,
            pick_file,
            get_sub_image,
            extract_image,
            extract_all,
        ])
        .run(tauri::generate_context!())
        .expect("启动 imsViewer 失败");
}
