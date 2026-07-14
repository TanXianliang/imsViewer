"""
IMS 图片打包工具
将多张图片打包为 .ims.{ext} 文件，标准查看器显示封面，专用浏览器可查看全部图片。
封面可以是 JPEG 或 PNG，保留原始格式写入（JPEG 的 EOI / PNG 的 IEND 标记之后的数据对标准查看器透明）。
"""

from __future__ import annotations

import argparse
import io
import os
import struct
import zlib
from pathlib import Path

from PIL import Image, ImageOps

# IMS 文件格式常量
MAGIC = b"IMSIMAGE"
VERSION = 1  # uint16 LE
HEADER_STRUCT = struct.Struct("<8s H I I")   # magic, version, image_count, data_start
FOOTER_OFFSET_STRUCT = struct.Struct("<I")    # index_offset
THUMBNAIL_MAGIC = b"IMSTHMB\0"
THUMBNAIL_VERSION = 1
THUMBNAIL_FORMAT_JPEG = 0
THUMBNAIL_MAX_SIZE = 160
THUMBNAIL_JPEG_QUALITY = 60
THUMBNAIL_TRAILER_STRUCT = struct.Struct("<8s H B B I I H H I I")


def create_embedded_thumbnail(cover_bytes: bytes) -> tuple[bytes, int, int]:
    """从封面生成适合远程目录预览的极小 JPEG 缩略图。"""
    with Image.open(io.BytesIO(cover_bytes)) as source:
        thumbnail = ImageOps.exif_transpose(source)
        resampling = getattr(Image, "Resampling", Image).LANCZOS
        thumbnail.thumbnail((THUMBNAIL_MAX_SIZE, THUMBNAIL_MAX_SIZE), resampling)

        if thumbnail.mode in ("RGBA", "LA") or "transparency" in thumbnail.info:
            rgba = thumbnail.convert("RGBA")
            flattened = Image.new("RGB", rgba.size, "white")
            flattened.paste(rgba, mask=rgba.getchannel("A"))
            thumbnail = flattened
        else:
            thumbnail = thumbnail.convert("RGB")

        output = io.BytesIO()
        thumbnail.save(
            output,
            format="JPEG",
            quality=THUMBNAIL_JPEG_QUALITY,
            optimize=True,
        )
        return output.getvalue(), thumbnail.width, thumbnail.height


def pack_images(image_paths: list[str], cover_index: int, output_path: str) -> None:
    """将多张图片打包为一个 IMS 文件。"""
    if cover_index < 0 or cover_index >= len(image_paths):
        raise ValueError(f"封面索引 {cover_index} 超出范围 (0~{len(image_paths) - 1})")

    # 1. 验证所有输入图片
    entries = []

    for i, path in enumerate(image_paths):
        if not os.path.isfile(path):
            raise FileNotFoundError(f"图片文件不存在: {path}")

        ext = Path(path).suffix.lower()
        if ext not in (".jpg", ".jpeg", ".png"):
            raise ValueError(f"不支持的图片格式 ({ext}): {path}，仅支持 .jpg / .jpeg / .png")

        stem = Path(path).stem.lower()
        if ".ims" in stem:
            raise ValueError(f"不允许使用 IMS 文件作为源图片 (防止嵌套): {path}")

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
    thumbnail_bytes, thumbnail_width, thumbnail_height = create_embedded_thumbnail(entries[0]["raw"])

    # 3. 构建索引条目数据
    entry_binaries: list[bytes] = []
    offset = 0

    for entry in entries:
        name_bytes = entry["filename"].encode("utf-8")
        entry_bin = (
            struct.pack("<H", len(name_bytes))
            + name_bytes
            + struct.pack("<B I I B", entry["format"], offset, len(entry["raw"]), 1 if entry["is_cover"] else 0)
        )
        entry_binaries.append(entry_bin)
        offset += len(entry["raw"])

    # 3. 构建索引区
    index_data = HEADER_STRUCT.pack(MAGIC, VERSION, len(entries), 0)
    for eb in entry_binaries:
        index_data += eb

    # 4. 计算缩略图和索引区偏移
    total_data_size = sum(len(e["raw"]) for e in entries)
    thumbnail_offset = total_data_size
    index_offset = thumbnail_offset + len(thumbnail_bytes)
    thumbnail_crc32 = zlib.crc32(thumbnail_bytes) & 0xffffffff
    thumbnail_trailer = THUMBNAIL_TRAILER_STRUCT.pack(
        THUMBNAIL_MAGIC,
        THUMBNAIL_VERSION,
        THUMBNAIL_FORMAT_JPEG,
        0,
        thumbnail_offset,
        len(thumbnail_bytes),
        thumbnail_width,
        thumbnail_height,
        thumbnail_crc32,
        0,
    )

    # 5. 写入输出文件
    with open(output_path, "wb") as f:
        for entry in entries:
            f.write(entry["raw"])
        f.write(thumbnail_bytes)
        f.write(index_data)
        f.write(thumbnail_trailer)
        f.write(FOOTER_OFFSET_STRUCT.pack(index_offset))

    print(f"打包完成: {output_path}")
    print(f"  共 {len(entries)} 张图片，封面: {entries[0]['filename']}")
    print(
        f"  [缩略图] {thumbnail_width}x{thumbnail_height} "
        f"({len(thumbnail_bytes)} bytes, offset={thumbnail_offset})"
    )
    for i, entry in enumerate(entries):
        tag = "[封面]" if entry["is_cover"] else f"[图片{i}]"
        print(f"  {tag} {entry['filename']} ({len(entry['raw'])} bytes)")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="IMS 图片打包工具 — 将多张图片打包为 .ims.{ext} 文件",
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
