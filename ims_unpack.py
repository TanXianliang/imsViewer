"""
IMS 图片解包工具
解析 .ims.{ext} 文件，列出/提取其中包含的所有图片。
"""

from __future__ import annotations

import argparse
import os
import struct
from pathlib import Path

# IMS 文件格式常量
MAGIC = b"IMSIMAGE"
# Header: magic(8) + version(2) + image_count(4) + data_start(4) = 18 bytes
HEADER_STRUCT = struct.Struct("<8s H I I")
HEADER_SIZE = HEADER_STRUCT.size
# 末尾 4 字节: index_offset
FOOTER_OFFSET_STRUCT = struct.Struct("<I")
# 单条 entry 的固定部分: name_len(2) + format(1) + data_offset(4) + data_length(4) + is_cover(1)
# name 为变长，需单独读取
ENTRY_FIXED_STRUCT = struct.Struct("<B I I B")  # format, data_offset, data_length, is_cover

FORMAT_NAMES = {0: "JPEG", 1: "PNG"}
FORMAT_EXTENSIONS = {0: ".jpg", 1: ".png"}


def parse_ims(filepath: str) -> dict:
    """解析 IMS 文件，返回其索引信息。

    返回: {
        "filepath": str,
        "file_size": int,
        "version": int,
        "image_count": int,
        "data_start": int,
        "index_offset": int,
        "images": [
            {
                "index": int,
                "filename": str,
                "format": int (0=JPEG, 1=PNG),
                "format_name": str,
                "data_offset": int,
                "data_length": int,
                "is_cover": bool,
            },
            ...
        ],
    }
    """
    if not os.path.isfile(filepath):
        raise FileNotFoundError(f"文件不存在: {filepath}")

    with open(filepath, "rb") as f:
        data = f.read()

    file_size = len(data)
    if file_size < 4:
        raise ValueError(f"文件太小，不是有效的 IMS 文件: {filepath}")

    # 1. 读取末尾 4 字节 → index_offset
    index_offset = FOOTER_OFFSET_STRUCT.unpack_from(data, file_size - 4)[0]

    if index_offset < 0 or index_offset >= file_size - 4:
        raise ValueError(f"索引偏移异常 ({index_offset})，不是有效的 IMS 文件")

    # 2. 从 index_offset 处读取 header
    magic, version, image_count, data_start = HEADER_STRUCT.unpack_from(data, index_offset)

    if magic != MAGIC:
        raise ValueError(f"Magic 不匹配: 期望 {MAGIC!r}，实际 {magic!r}")

    if version != 1:
        raise ValueError(f"不支持的 IMS 版本: {version}")

    # 3. 遍历 entries
    pos = index_offset + HEADER_SIZE
    images = []

    for i in range(image_count):
        if pos + 2 > file_size:
            raise ValueError(f"文件损坏: 无法读取图片 {i} 的名称长度")

        name_len = struct.unpack_from("<H", data, pos)[0]
        pos += 2

        if pos + name_len > file_size:
            raise ValueError(f"文件损坏: 无法读取图片 {i} 的名称")

        name = data[pos:pos + name_len].decode("utf-8")
        pos += name_len

        if pos + ENTRY_FIXED_STRUCT.size > file_size:
            raise ValueError(f"文件损坏: 无法读取图片 {i} 的条目信息")

        fmt, data_offset, data_length, is_cover_byte = ENTRY_FIXED_STRUCT.unpack_from(data, pos)
        pos += ENTRY_FIXED_STRUCT.size

        images.append({
            "index": i,
            "filename": name,
            "format": fmt,
            "format_name": FORMAT_NAMES.get(fmt, f"未知({fmt})"),
            "data_offset": data_offset,
            "data_length": data_length,
            "is_cover": bool(is_cover_byte),
        })

    return {
        "filepath": filepath,
        "file_size": file_size,
        "version": version,
        "image_count": image_count,
        "data_start": data_start,
        "index_offset": index_offset,
        "images": images,
    }


def extract_image(filepath: str, image_info: dict, output_dir: str) -> str:
    """从 IMS 文件中提取单张图片。

    返回: 输出文件路径
    """
    ext = FORMAT_EXTENSIONS.get(image_info["format"], ".bin")
    out_name = Path(image_info["filename"]).stem + ext
    out_path = os.path.join(output_dir, out_name)

    # 处理重名
    counter = 1
    base, ext = os.path.splitext(out_name)
    while os.path.exists(out_path):
        out_name = f"{base}_{counter}{ext}"
        out_path = os.path.join(output_dir, out_name)
        counter += 1

    os.makedirs(output_dir, exist_ok=True)

    with open(filepath, "rb") as f:
        f.seek(image_info["data_offset"])
        raw = f.read(image_info["data_length"])

    with open(out_path, "wb") as f:
        f.write(raw)

    return out_path


def list_images(info: dict) -> None:
    """打印 IMS 文件中的图片列表。"""
    print(f"文件: {info['filepath']}")
    print(f"大小: {info['file_size']:,} bytes")
    print(f"版本: {info['version']}")
    print(f"图片数量: {info['image_count']}")
    print()
    print(f"{'索引':>4}  {'封面':>4}  {'格式':>6}  {'偏移':>10}  {'大小':>10}  文件名")
    print("-" * 70)
    for img in info["images"]:
        cover_tag = "★" if img["is_cover"] else ""
        print(
            f"{img['index']:>4}  {cover_tag:>4}  "
            f"{img['format_name']:>6}  "
            f"{img['data_offset']:>10}  "
            f"{img['data_length']:>10}  "
            f"{img['filename']}"
        )


def main() -> None:
    parser = argparse.ArgumentParser(
        description="IMS 图片解包工具 — 列出/提取 .ims.{ext} 文件中的图片",
    )
    parser.add_argument(
        "input", help="输入的 .ims.{ext} 文件路径",
    )
    parser.add_argument(
        "-l", "--list", action="store_true",
        help="仅列出文件中的图片信息，不提取",
    )
    parser.add_argument(
        "-a", "--extract-all", action="store_true",
        help="提取所有图片",
    )
    parser.add_argument(
        "-e", "--extract", type=int, nargs="+", metavar="N",
        help="按索引提取指定图片 (从 0 开始，可多个)",
    )
    parser.add_argument(
        "--cover", action="store_true",
        help="仅提取封面图片",
    )
    parser.add_argument(
        "-o", "--output", default=None,
        help="输出目录 (默认: 与输入文件同目录)",
    )

    args = parser.parse_args()

    try:
        info = parse_ims(args.input)
    except (FileNotFoundError, ValueError) as e:
        print(f"错误: {e}")
        return

    output_dir = args.output or os.path.dirname(os.path.abspath(args.input))

    # 默认无操作时，显示列表
    no_action = not (args.list or args.extract_all or args.extract or args.cover)

    if args.list or no_action:
        list_images(info)
        if no_action:
            print("\n提示: 使用 -a 提取全部，-e N 提取指定图片，--cover 提取封面")
            return

    # 收集要提取的图片索引
    to_extract: set[int] = set()

    if args.extract_all:
        to_extract.update(range(info["image_count"]))
    if args.extract:
        for n in args.extract:
            if n < 0 or n >= info["image_count"]:
                print(f"警告: 索引 {n} 超出范围 (0~{info['image_count'] - 1})，已忽略")
            else:
                to_extract.add(n)
    if args.cover:
        for img in info["images"]:
            if img["is_cover"]:
                to_extract.add(img["index"])

    if not to_extract:
        return

    print(f"提取到: {output_dir}")
    for idx in sorted(to_extract):
        img = info["images"][idx]
        out_path = extract_image(args.input, img, output_dir)
        cover_tag = " [封面]" if img["is_cover"] else ""
        print(f"  [{idx}] {img['filename']} → {os.path.basename(out_path)}{cover_tag}")

    print(f"\n完成: 已提取 {len(to_extract)} 张图片。")


if __name__ == "__main__":
    main()
