from __future__ import annotations

import argparse
from pathlib import Path

import numpy as np
import torch


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Convert the official SSCD TorchScript model to ONNX")
    parser.add_argument("input", type=Path, help="Path to sscd_disc_mixup.torchscript.pt")
    parser.add_argument("output", type=Path, help="Destination ONNX path")
    parser.add_argument("--opset", type=int, default=17)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)

    model = torch.jit.load(str(args.input), map_location="cpu").eval()
    sample = torch.linspace(0.0, 1.0, 3 * 320 * 320, dtype=torch.float32).reshape(1, 3, 320, 320)

    with torch.inference_mode():
        reference = model(sample).detach().cpu().numpy()

    torch.onnx.export(
        model,
        sample,
        str(args.output),
        export_params=True,
        opset_version=args.opset,
        do_constant_folding=True,
        input_names=["input"],
        output_names=["embedding"],
        dynamic_axes={"input": {0: "batch"}, "embedding": {0: "batch"}},
    )

    np.save(args.output.with_suffix(".reference.npy"), reference)
    print(f"wrote {args.output}")
    print(f"reference_shape={reference.shape} norm={np.linalg.norm(reference[0]):.8f}")


if __name__ == "__main__":
    main()
