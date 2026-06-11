"""验证 IMS 文件格式是否正确"""
import os
import struct
import sys

MAGIC = b"IMSIMAGE"

with open("test_output.ims.jpg", "rb") as f:
    data = f.read()

file_size = len(data)

# 读取末尾 4 字节 → index_offset
index_offset = struct.unpack_from("<I", data, file_size - 4)[0]
print(f"文件大小: {file_size} bytes")
print(f"索引偏移: {index_offset}")

# 验证 magic
magic = data[index_offset:index_offset + 8]
assert magic == MAGIC, f"Magic 不匹配: {magic}"
print(f"Magic 正确: {magic}")

# 读取 header
version, image_count, data_start = struct.unpack_from("<H I I", data, index_offset + 8)
print(f"版本: {version}, 图片数量: {image_count}, 数据起始: {data_start}")

# 读取 entries
pos = index_offset + 8 + 2 + 4 + 4  # magic(8) + version(2) + count(4) + data_start(4) = 18
for i in range(image_count):
    name_len = struct.unpack_from("<H", data, pos)[0]
    pos += 2
    name = data[pos:pos + name_len].decode("utf-8")
    pos += name_len
    fmt, data_off, data_len, is_cover_byte = struct.unpack_from("<B I I B", data, pos)
    pos += 1 + 4 + 4 + 1
    is_cover = bool(is_cover_byte)
    fmt_name = "JPEG" if fmt == 0 else "PNG"
    cover_tag = " [封面]" if is_cover else ""
    # 验证提取的数据
    extracted = data[data_off:data_off + data_len]
    valid = extracted[:2] == b'\xff\xd8' if fmt == 0 else extracted[:8] == b'\x89PNG\r\n\x1a\n'
    print(f"  图片{i}: {name} | {fmt_name} | offset={data_off} len={data_len} | cover={is_cover} | 有效={valid}")

# 验证封面数据是有效 JPEG
cover_start = data[0:2]
assert cover_start == b'\xff\xd8', f"文件开头不是 JPEG SOI 标记: {cover_start.hex()}"
print(f"\n封面 JPEG SOI 标记正确: {cover_start.hex()}")

print("\n所有验证通过!")