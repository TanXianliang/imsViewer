// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // 读取命令行参数：右键"使用 imsViewer 打开"传入的文件路径
    let args: Vec<String> = std::env::args().collect();
    let launch_file = if args.len() > 1 { Some(args[1].clone()) } else { None };
    imsviewer_lib::run(launch_file)
}
