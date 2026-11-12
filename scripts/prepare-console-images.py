"""将官方透明主机原图整理为共用高清资源。"""

import argparse
import hashlib
import json
import shutil
from pathlib import Path

from PIL import Image, ImageChops, ImageFilter, ImageOps

ROOT = Path(__file__).resolve().parent.parent
CANVAS_SIZE = 1536
SUBJECT_LIMIT = 1320
ALPHA_CORE_THRESHOLD = 248


def prepare(path):
    image = Image.open(path).convert("RGBA")
    alpha = image.getchannel("A")
    if alpha.getextrema() != (0, 255):
        raise ValueError(f"需要带完整透明通道的原图：{path}")
    # 官方广告图的地面长投影由页面投影替代；保留主体外侧两像素的原始抗锯齿。
    core = alpha.point(lambda value: 255 if value >= ALPHA_CORE_THRESHOLD else 0)
    image.putalpha(ImageChops.multiply(alpha, core.filter(ImageFilter.MaxFilter(5))))
    bounds = image.getchannel("A").getbbox()
    subject = image.crop(bounds)
    limit = min(SUBJECT_LIMIT, max(subject.size))
    subject = ImageOps.contain(subject, (limit, limit), Image.Resampling.LANCZOS)
    result = Image.new("RGBA", (CANVAS_SIZE, CANVAS_SIZE))
    result.alpha_composite(subject, ((CANVAS_SIZE - subject.width) // 2, (CANVAS_SIZE - subject.height) // 2))
    return result, {
        "sourceSize": list(image.size),
        "sourceSha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "cropBox": list(bounds),
        "subjectSize": list(subject.size),
        "outputSize": list(result.size),
        "alphaCoreThreshold": ALPHA_CORE_THRESHOLD,
        "edgePadding": 2,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-x", required=True, type=Path)
    parser.add_argument("--source-s", required=True, type=Path)
    parser.add_argument("--sync-ios-assets", action="store_true")
    args = parser.parse_args()
    output = ROOT / "src/assets/console"
    records = {}
    for model, source in [("x", args.source_x), ("s", args.source_s)]:
        image, info = prepare(source)
        target = output / f"series-{model}-transparent.png"
        image.save(target, optimize=True)
        info["outputSha256"] = hashlib.sha256(target.read_bytes()).hexdigest()
        info["outputBytes"] = target.stat().st_size
        records[model] = info
        if model == "x":
            image.save(output / "series-x.png", optimize=True)
        else:
            stock = Image.new("RGBA", image.size, "white")
            stock.alpha_composite(image)
            stock.convert("RGB").save(output / "series-s.jpeg", quality=95, subsampling=0, optimize=True)

        if args.sync_ios_assets:
            folder = ROOT / "iosapp/XBXRC/Resources/Assets.xcassets" / f"ConsoleSeries{model.upper()}.imageset"
            name = f"series-{model}.png"
            shutil.copy2(target, folder / name)
            contents_path = folder / "Contents.json"
            contents = json.loads(contents_path.read_text())
            for item in contents["images"]:
                if item.get("filename"):
                    previous = folder / item["filename"]
                    item["filename"] = name
                    if previous.name != name and previous.exists():
                        previous.unlink()
            contents_path.write_text(json.dumps(contents, ensure_ascii=False, indent=2, separators=(",", " : ")) + "\n")
    print(json.dumps(records, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
