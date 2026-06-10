from PIL import Image
import os

os.makedirs("test_images", exist_ok=True)

Image.new("RGB", (100, 100), "red").save("test_images/0.jpg", "JPEG")
Image.new("RGB", (100, 100), "green").save("test_images/1.png", "PNG")
Image.new("RGB", (100, 100), "blue").save("test_images/2.jpg", "JPEG")

for f in os.listdir("test_images"):
    path = os.path.join("test_images", f)
    print(f"  {f} ({os.path.getsize(path)} bytes)")
print("测试图片已创建")