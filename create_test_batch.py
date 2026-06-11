from PIL import Image
import os

d = "test_batch"

# ab 组: ab.jpg 封面 + 3 张 PNG 子图
Image.new("RGB", (100, 100), "red").save(f"{d}/ab.jpg", "JPEG")
Image.new("RGB", (100, 100), "green").save(f"{d}/ab_00001_.png", "PNG")
Image.new("RGB", (100, 100), "blue").save(f"{d}/ab_00002_.png", "PNG")
Image.new("RGB", (100, 100), "yellow").save(f"{d}/ab_00003_.png", "PNG")

# xxx 组: xxx.png 封面 + 5 张混合格式子图
Image.new("RGB", (100, 100), "cyan").save(f"{d}/xxx.png", "PNG")
Image.new("RGB", (100, 100), "magenta").save(f"{d}/xxx_00001_.png", "PNG")
Image.new("RGB", (100, 100), "white").save(f"{d}/xxx_00002_.jpg", "JPEG")
Image.new("RGB", (100, 100), "black").save(f"{d}/xxx_00003_.png", "PNG")
Image.new("RGB", (100, 100), "gray").save(f"{d}/xxx_00005_.png", "PNG")
Image.new("RGB", (100, 100), "orange").save(f"{d}/xxx_00189_.jpg", "JPEG")

# 孤立的子图（无封面），应被跳过
Image.new("RGB", (100, 100), "pink").save(f"{d}/lonely_00001_.png", "PNG")

# 孤立的封面（无子图），应被跳过
Image.new("RGB", (100, 100), "brown").save(f"{d}/single.jpg", "JPEG")

for f in sorted(os.listdir(d)):
    path = os.path.join(d, f)
    print(f"  {f} ({os.path.getsize(path)} bytes)")