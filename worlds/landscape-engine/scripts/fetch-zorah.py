#!/usr/bin/env python3
"""Downloads and unpacks NVIDIA's Zorah scene, which is not stored in this repository.

It is 75 GB to download and 83 GB unpacked, and is MIT licensed by NVIDIA (docs/zorah.md).
The download resumes if it is interrupted. Unpacking needs a `tar` that reads 7z archives, as
Windows' own does.

    python scripts/fetch-zorah.py D:/SolsticeAssets/nvidia/zorah

Then bake it. A region is much quicker than the whole scene:

    bake --large-gltf DIR/unpacked/zorah_textured_public.v1/zorah_textured_public.v1.gltf \\
         --region 7.73,2.28,-11.15,5 --output zorah.solstice
"""
import os
import subprocess
import sys

URL = "https://developer.download.nvidia.com/ProGraphics/nvpro-samples/zorah_textured_public.v1.7z"
BYTES = 75_152_910_406
GLTF = "zorah_textured_public.v1/zorah_textured_public.v1.gltf"


def main():
    root = sys.argv[1]
    os.makedirs(root, exist_ok=True)
    archive = os.path.join(root, os.path.basename(URL))
    if not os.path.exists(archive) or os.path.getsize(archive) != BYTES:
        subprocess.run(
            ["curl", "-L", "-C", "-", "--retry", "50", "--retry-all-errors", "--retry-delay", "30",
             "--fail", "-o", archive, URL],
            check=True,
        )
    if os.path.getsize(archive) != BYTES:
        sys.exit(f"{archive} is not the expected {BYTES} bytes")
    unpacked = os.path.join(root, "unpacked")
    if not os.path.exists(os.path.join(unpacked, GLTF)):
        os.makedirs(unpacked, exist_ok=True)
        subprocess.run(["tar", "-xf", archive, "-C", unpacked], check=True)
    print(os.path.join(unpacked, GLTF))


if __name__ == "__main__":
    main()
