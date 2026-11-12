"""核验实际图片、矢量文件及平台图标容器，并保存全量资源清单。"""

import hashlib
import json
import subprocess
import xml.etree.ElementTree as ET
from pathlib import Path

from PIL import Image

EVIDENCE = Path(__file__).resolve().parent
ROOT = EVIDENCE.parents[2]
SUFFIXES = {".png", ".jpg", ".jpeg", ".svg", ".ico", ".icns"}
checks = []
before = {item["path"]: item for item in json.loads((EVIDENCE / "assets-before.json").read_text())}


def check(condition, label):
    checks.append({"label": label, "passed": bool(condition)})


def save(name, value):
    (EVIDENCE / name).write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


paths = subprocess.check_output([
    "rg", "--files", "src/assets", "src-tauri/icons", "public",
    "iosapp/XBXRC/Resources/Assets.xcassets",
], cwd=ROOT, text=True).splitlines()
assets = []
containers = {}
for name in sorted(paths):
    path = ROOT / name
    if path.suffix not in SUFFIXES:
        continue
    info = {"path": name, "bytes": path.stat().st_size, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
    if path.suffix == ".svg":
        svg = ET.parse(path).getroot()
        info["viewBox"] = svg.get("viewBox")
        info["embeddedImages"] = len(svg.findall(".//{http://www.w3.org/2000/svg}image"))
        check(info["viewBox"] and info["embeddedImages"] == 0, f"{name} 使用纯矢量及有效viewBox")
        if name.startswith(("src/assets/nav/", "src/assets/stream/")):
            check(svg.get("width") == "32" and svg.get("height") == "32"
                  and svg.get("preserveAspectRatio") == "xMidYMid meet"
                  and svg.get("aria-hidden") == "true" and svg.get("class") is None,
                  f"{name} 统一矢量尺寸与比例")
    else:
        with Image.open(path) as image:
            image.load()
            info.update(size=list(image.size), mode=image.mode)
            if image.mode == "RGBA":
                alpha = image.getchannel("A")
                info.update(alphaLevels=sum(value > 0 for value in alpha.histogram()), visibleBox=alpha.getbbox())
            if path.suffix == ".ico":
                sizes = sorted(image.ico.sizes())
                for size in sizes:
                    frame = image.ico.getimage(size)
                    frame.load()
                    check(frame.size == size and frame.mode == "RGBA", f"ICO {size}帧有效")
                containers["icoSizes"] = sizes
                check(sizes == [(size, size) for size in [16, 24, 32, 48, 64, 256]], "ICO 原版六个尺寸")
            elif path.suffix == ".icns":
                frames = image.info["sizes"]
                for size in frames:
                    frame = image.icns.getimage(size)
                    frame.load()
                    check(frame.size == (size[0] * size[2], size[1] * size[2]), f"ICNS {size}帧有效")
                containers["icnsSizes"] = frames
                check(max(size[0] * size[2] for size in frames) == 1024, "ICNS 包含1024px图")
    assets.append(info)

by_path = {item["path"]: item for item in assets}
original_icons = {name: info for name, info in before.items() if name.startswith("src-tauri/icons/")}
exports = {name: info for name, info in original_icons.items() if name.endswith(".png")}
check(len(exports) == 50, "50张原版平台PNG完整")
check(set(original_icons) == {name for name in by_path if name.startswith("src-tauri/icons/")},
      "应用图标文件清单与原版一致")
for name, original in original_icons.items():
    info = by_path.get(name)
    check(info is not None and info["sha256"] == original["sha256"], f"{name} 内容与原版一致")
    if name.endswith(".png"):
        check(info is not None and info["size"] == original["size"] and info["mode"] == original["mode"],
              f"{name} 尺寸与颜色模式保持原版")

console_generation = json.loads((EVIDENCE / "console-generation.json").read_text())
for model in ["x", "s"]:
    info = by_path[f"src/assets/console/series-{model}-transparent.png"]
    check(info["size"] == [1536, 1536] and info["mode"] == "RGBA" and info["alphaLevels"] >= 32,
          f"Series {model.upper()} 高清透明抗锯齿有效")
    check(info["sha256"] == console_generation[model]["outputSha256"], f"Series {model.upper()} 与生成记录一致")
    ios = by_path[f"iosapp/XBXRC/Resources/Assets.xcassets/ConsoleSeries{model.upper()}.imageset/series-{model}.png"]
    check(ios["sha256"] == info["sha256"], f"Series {model.upper()} 存量资源同步")

for contents in sorted((ROOT / "iosapp/XBXRC/Resources/Assets.xcassets").glob("*/Contents.json")):
    for image in json.loads(contents.read_text()).get("images", []):
        if image.get("filename"):
            check((contents.parent / image["filename"]).is_file(), f"{contents.parent.name} 引用文件存在")

check(by_path["src/assets/ctrl/series-ctrl-transparent.png"]["size"] == [1488, 1032], "高清手柄图完整")
check(by_path["src/assets/ctrl/series-ctrl.jpeg"]["size"] == [1488, 1488], "手柄兼容图同步高清")
check(by_path["src/assets/main-bg.jpg"]["size"] == [3840, 1402], "4K背景原图完整")

changed = [name for name in by_path if name in before and by_path[name]["sha256"] != before[name]["sha256"]]
added = [name for name in by_path if name not in before]
removed = [name for name in before if name not in by_path]
save("assets-after.json", assets)
summary = {
    "assetCount": len(assets), "svgCount": sum(name.endswith(".svg") for name in by_path),
    "platformPngCount": len(exports), "passed": sum(item["passed"] for item in checks),
    "failed": sum(not item["passed"] for item in checks), "containers": containers,
    "changed": changed, "added": added, "removed": removed, "checks": checks,
}
save("asset-results.json", summary)
print(json.dumps({key: summary[key] for key in ["assetCount", "svgCount", "platformPngCount", "passed", "failed", "containers"]}, ensure_ascii=False, indent=2))
raise SystemExit(1 if summary["failed"] else 0)
