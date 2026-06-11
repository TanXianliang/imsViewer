"""
IMS 批量打包工具
按命名规则自动扫描目录，将匹配的图片组打包为 .ims.{ext} 文件。

规则：
  - 封面 = 文件名为 {base}.{ext}（不含 _XXXXX_ 模式）
  - 子图 = 文件名匹配 {base}_XXXXX_.{ext}（XXXXX 为 5 位数字）
  - 输出 = {base}.ims.{ext}

示例：
  目录中有 ab.jpg, ab_00001_.png, ab_00002_.png
  → 自动生成 ab.ims.jpg（封面 ab.jpg + 2 张子图）
"""

from __future__ import annotations

import argparse
import os
import re
from collections import defaultdict
from pathlib import Path

from ims_pack import pack_images

# 匹配子图命名: {base}_XXXXX_
RELATED_PATTERN = re.compile(r"^(.+)_(\d{5})_$")

# 支持的图片扩展名
VALID_EXTS = {".jpg", ".jpeg", ".png"}


def is_ims_file(path: Path) -> bool:
    """检查文件名是否包含 .ims（防止嵌套打包）。"""
    stem = path.stem.lower()
    return ".ims" in stem


def scan_directory(directory: str) -> dict[str, dict]:
    """扫描目录，按命名规则分组图片。

    返回: {base_name: {"cover": Path, "related": [(seq, Path), ...]}}
    """
    groups: dict[str, dict] = defaultdict(lambda: {"cover": None, "related": []})

    for entry in sorted(os.listdir(directory)):
        filepath = Path(directory) / entry
        if not filepath.is_file():
            continue

        ext = filepath.suffix.lower()
        if ext not in VALID_EXTS:
            continue

        if is_ims_file(filepath):
            print(f"  [跳过 IMS 文件] {entry}")
            continue

        stem = filepath.stem

        m = RELATED_PATTERN.match(stem)
        if m:
            base_name = m.group(1)
            seq = int(m.group(2))
            groups[base_name]["related"].append((seq, filepath))
        else:
            base_name = stem
            groups[base_name]["cover"] = filepath

    return groups


def batch_pack(directory: str) -> None:
    """扫描目录并批量打包所有匹配的图片组。"""
    groups = scan_directory(directory)

    packed_count = 0
    for base_name in sorted(groups.keys()):
        group = groups[base_name]
        cover = group["cover"]
        related = group["related"]

        if cover is None:
            # 有子图但没有封面，跳过
            names = [p.name for _, p in related]
            print(f"[跳过] 缺少封面 '{base_name}.*'，但找到子图: {names}")
            continue

        if not related:
            continue  # 有封面但没有子图，跳过

        # 按序号排序子图
        related.sort(key=lambda x: x[0])

        cover_ext = cover.suffix.lower()

        # 收集所有图片路径：封面 + 子图
        image_paths = [str(cover)] + [str(p) for _, p in related]
        output_path = os.path.join(directory, f"{base_name}.ims{cover_ext}")

        print(f"\n--- 打包: {base_name}.ims{cover_ext} ---")
        try:
            pack_images(image_paths, cover_index=0, output_path=output_path)
            packed_count += 1
        except (FileNotFoundError, ValueError) as e:
            print(f"  打包失败: {e}")

    if packed_count == 0:
        print("\n未找到可打包的图片组。")
    else:
        print(f"\n批量打包完成，共生成 {packed_count} 个 IMS 文件。")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="IMS 批量打包工具 — 按命名规则自动扫描目录并打包图片组",
    )
    parser.add_argument(
        "directory", nargs="?", default=".",
        help="要扫描的目录路径 (默认: 当前目录)",
    )

    args = parser.parse_args()
    directory = os.path.abspath(args.directory)

    if not os.path.isdir(directory):
        print(f"错误: 目录不存在: {directory}")
        return

    print(f"扫描目录: {directory}")
    batch_pack(directory)


if __name__ == "__main__":
    main()