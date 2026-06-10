"""
IMS 图片打包工具
将多张图片打包为 .ims.jpg 文件，标准查看器显示封面，专用浏览器可查看全部图片。
"""

from __future__ import annotations

import argparse
import os
import struct
from io import BytesIO
from pathlib import Path

from PIL import Image

# IMS 文件格式常量
MAGIC = b"IMSIMAGE"
VERSION = 1  # uint16 LE
ENTRY_STRUCT = struct.Struct("<H 1s I I ?")  # name_len, format, data_offset, data_length, is_cover
HEADER_STRUCT = struct.Struct("<8s H I I")   # magic, version, image_count, data_start
FOOTER_OFFSET_STRUCT = struct.Struct("<I")    # index_offset


def pack_images(image_paths: list[str], cover_index: int, output_path: str) -> None:
    """将多张图片打包为一个 IMS 文件。"""
    if cover_index < 0 or cover_index >= len(image_paths):
        raise ValueError(f"封面索引 {cover_index} 超出范围 (0~{len(image_paths) - 1})")

    # 1. 验证所有输入图片
    entries = []
    raw_data_list: list[bytes] = []

    for i, path in enumerate(image_paths):
        if not os.path.isfile(path):
            raise FileNotFoundError(f"图片文件不存在: {path}")

        ext = Path(path).suffix.lower()
        if ext not in (".jpg", ".jpeg", ".png"):
            raise ValueError(f"不支持的图片格式 ({ext}): {path}，仅支持 .jpg / .jpeg / .png")

        with open(path, "rb") as f:
            raw = f.read()

        fmt = 0 if ext in (".jpg", ".jpeg") else 1  # 0=JPEG, 1=PNG
        filename = os.path.basename(path)

        entries.append({
            "filename": filename,
            "format": fmt,
            "raw": raw,
            "is_cover": (i == cover_index),
        })

    # 2. 将 entries 按封面在前重排
    entries.sort(key=lambda e: not e["is_cover"])  # 封面排第一

    # 3. 封面如果不是 JPEG，先转为 JPEG
    cover_entry = entries[0]
    if cover_entry["format"] != 0:  # PNG 封面需转换
        img = Image.open(BytesIO(cover_entry["raw"]))
        buf = BytesIO()
        rgb_img = img.convert("RGB")
        rgb_img.save(buf, format="JPEG", quality=95)
        cover_entry["raw"] = buf.getvalue()
        # 注意：format 字段保留原始格式信息，只转换实际存储数据

    # 4. 构建索引条目数据
    entry_binaries: list[bytes] = []
    offset = 0

    for entry in entries:
        name_bytes = entry["filename"].encode("utf-8")
        entry_bin = (
            struct.pack("<H", len(name_bytes))
            + name_bytes
            + struct.pack("<B I I ?", entry["format"], offset, len(entry["raw"]), entry["is_cover"])
        )
        entry_binaries.append(entry_bin)
        offset += len(entry["raw"])

    # 5. 构建索引区
    index_data = HEADER_STRUCT.pack(MAGIC, VERSION, len(entries), 0)
    for eb in entry_binaries:
        index_data += eb

    # 6. 计算索引区偏移（先写入所有图片数据，再写入索引区）
    total_data_size = sum(len(e["raw"]) for e in entries)
    index_offset = total_data_size

    # 7. 写入输出文件
    with open(output_path, "wb") as f:
        for entry in entries:
            f.write(entry["raw"])
        f.write(index_data)
        f.write(FOOTER_OFFSET_STRUCT.pack(index_offset))

    print(f"打包完成: {output_path}")
    print(f"  共 {len(entries)} 张图片，封面: {entries[0]['filename']}")
    for i, entry in enumerate(entries):
        tag = "[封面]" if entry["is_cover"] else f"[图片{i}]"
        print(f"  {tag} {entry['filename']} ({len(entry['raw'])} bytes)")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="IMS 图片打包工具 — 将多张图片打包为 .ims.jpg 文件",
    )
    parser.add_argument(
        "images", nargs="+", help="输入图片路径列表 (.jpg / .jpeg / .png)"
    )
    parser.add_argument(
        "--cover", type=int, default=0,
        help="封面图片索引，从 0 开始 (默认: 0)",
    )
    parser.add_argument(
        "-o", "--output", default="output.ims.jpg",
        help="输出文件路径 (默认: output.ims.jpg)",
    )

    args = parser.parse_args()
    try:
        pack_images(args.images, args.cover, args.output)
    except (FileNotFoundError, ValueError) as e:
        print(f"错误: {e}")


if __name__ == "__main__":
    main()