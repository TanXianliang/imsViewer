"""验证 IMS 文件索引、图片范围和内嵌缩略图。"""
from __future__ import annotations

import argparse
import zlib

from ims_unpack import parse_ims


def verify(filepath: str) -> None:
    info = parse_ims(filepath)
    print(f"文件大小: {info['file_size']:,} bytes")
    print(f"版本: {info['version']}, 图片数量: {info['image_count']}")
    print(f"索引偏移: {info['index_offset']}")

    with open(filepath, "rb") as file:
        data = file.read()

    covers = 0
    thumbnail = info.get("thumbnail")
    image_data_end = thumbnail["data_offset"] if thumbnail else info["index_offset"]
    for image in info["images"]:
        start = image["data_offset"]
        end = start + image["data_length"]
        assert end <= image_data_end, f"图片 {image['filename']} 数据越界"
        extracted = data[start:end]
        if image["format"] == 0:
            valid = extracted.startswith(b"\xff\xd8")
        elif image["format"] == 1:
            valid = extracted.startswith(b"\x89PNG\r\n\x1a\n")
        else:
            valid = bool(extracted)
        assert valid, f"图片 {image['filename']} 格式签名无效"
        covers += int(image["is_cover"])
        print(
            f"  图片{image['index']}: {image['filename']} | "
            f"offset={start} len={image['data_length']} | cover={image['is_cover']}"
        )

    assert covers == 1, f"封面数量应为 1，实际为 {covers}"

    if thumbnail:
        start = thumbnail["data_offset"]
        end = start + thumbnail["data_length"]
        assert end == info["index_offset"], "缩略图应紧邻 IMS 索引"
        thumbnail_bytes = data[start:end]
        assert thumbnail_bytes.startswith(b"\xff\xd8"), "内嵌缩略图不是 JPEG"
        actual_crc32 = zlib.crc32(thumbnail_bytes) & 0xffffffff
        assert actual_crc32 == thumbnail["crc32"], "内嵌缩略图 CRC32 不匹配"
        print(
            f"内嵌缩略图: {thumbnail['width']}x{thumbnail['height']} | "
            f"offset={start} len={thumbnail['data_length']} | CRC32 通过"
        )
    else:
        print("内嵌缩略图: 无（兼容旧 IMS）")

    print("所有验证通过!")


def main() -> None:
    parser = argparse.ArgumentParser(description="验证 IMS 文件格式")
    parser.add_argument("input", nargs="?", default="test_output.ims.jpg")
    args = parser.parse_args()
    verify(args.input)


if __name__ == "__main__":
    main()
