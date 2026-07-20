from __future__ import annotations

import argparse
import hashlib
import json
import time
from pathlib import Path

import numpy as np
import onnx
import onnxruntime as ort
import torch


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Verify SSCD TorchScript and ONNX parity")
    parser.add_argument("torchscript", type=Path)
    parser.add_argument("onnx", type=Path)
    parser.add_argument("--metadata", type=Path)
    return parser.parse_args()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> None:
    args = parse_args()
    onnx.checker.check_model(onnx.load(str(args.onnx)))

    sample = torch.linspace(0.0, 1.0, 3 * 320 * 320, dtype=torch.float32).reshape(1, 3, 320, 320)
    torch_model = torch.jit.load(str(args.torchscript), map_location="cpu").eval()

    torch_start = time.perf_counter()
    with torch.inference_mode():
        torch_output = torch_model(sample).detach().cpu().numpy()
    torch_ms = (time.perf_counter() - torch_start) * 1000

    providers = ["CPUExecutionProvider"]
    session = ort.InferenceSession(str(args.onnx), providers=providers)
    ort_start = time.perf_counter()
    ort_output = session.run(None, {"input": sample.numpy()})[0]
    ort_ms = (time.perf_counter() - ort_start) * 1000

    cosine = float(np.dot(torch_output[0], ort_output[0]) / (np.linalg.norm(torch_output[0]) * np.linalg.norm(ort_output[0])))
    max_abs = float(np.max(np.abs(torch_output - ort_output)))
    if torch_output.shape != (1, 512) or ort_output.shape != (1, 512):
        raise SystemExit(f"unexpected output shapes: torch={torch_output.shape}, onnx={ort_output.shape}")
    if cosine < 0.99999:
        raise SystemExit(f"cosine parity check failed: {cosine}")

    metadata = {
        "model_id": "sscd_disc_mixup",
        "model_version": 1,
        "embedding_dim": 512,
        "input_width": 320,
        "input_height": 320,
        "preprocess_version": 1,
        "onnx_sha256": sha256_file(args.onnx),
        "torchscript_sha256": sha256_file(args.torchscript),
        "parity_cosine": cosine,
        "max_abs_error": max_abs,
        "torchscript_cpu_ms": torch_ms,
        "onnx_cpu_ms": ort_ms,
    }

    if args.metadata:
        args.metadata.parent.mkdir(parents=True, exist_ok=True)
        args.metadata.write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    print(json.dumps(metadata, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
