#!/usr/bin/env python3
"""从冻结 TAK schema/图集生成公共协议合同与 iOS 图集资源。"""

import argparse
import base64
import hashlib
import json
import re
import zipfile
from pathlib import Path


REPO = Path(__file__).resolve().parents[2]
DEFAULT_SOURCE = REPO / "dev-docs/validation/2026-11-26-ios-tak-protocol-engine-plan"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=DEFAULT_SOURCE)
    args = parser.parse_args()
    inventory = json.loads((args.source / "protocol-inventory.json").read_text())
    manifest = json.loads((args.source / "official-icon-atlas-source.json").read_text())
    atlas_source = next(item for item in manifest["files"] if item["dpi"] == 3)
    with zipfile.ZipFile(args.source / manifest["archive"]) as source:
        raw = source.read(atlas_source["file"])
    if hashlib.sha256(raw).hexdigest() != atlas_source["sha256"]:
        raise ValueError("TAK 图集来源哈希不一致")
    literal = re.search(r"JSON\.parse\('((?:\\.|[^'\\])*)'\)", raw.decode()).group(1)
    atlas = json.loads(literal)
    image = base64.b64decode(atlas["meta"]["image"].split(",", 1)[1], validate=True)
    layout = next(item for item in inventory["schemas"] if item["path"].endswith("layout-schema.json"))
    colors = {
        "system_contentPrimary": "#ffffffff", "system_contentSecondary": "#ffffff18",
        "system_contrastPrimary": "#000000ff", "system_contrastSecondary": "#808080ff",
        "system_actionColorDefault": "#ffffff", "system_actionColorA": "#61ba75",
        "system_actionColorB": "#db6b6b", "system_actionColorX": "#619ed9",
        "system_actionColorY": "#d9ba5c", "system_accentPrimary": "#4fb07a",
        "system_accentSecondary": "#397d57",
    }
    contract = {
        "irVersion": 1,
        "compilerRevision": 1,
        "schema": layout["contract"],
        "iconNames": sorted(atlas["frames"]),
        "systemColors": colors,
        "controlDefinitions": {
            "button": "_Button", "joystick": "_Joystick", "directionalPad": "_DirectionalPad",
            "touchpad": "_Touchpad", "throttle": "_Throttle", "arcadeButtons": "_ArcadeButtons",
        },
        "controlStyles": {
            "button": "ButtonStyles", "joystick": "JoystickStyles",
            "directionalPad": "DirectionalPadStyles", "touchpad": "TouchpadStyles",
            "throttle": "ThrottleStyles", "arcadeButtons": "ButtonStyles",
        },
        "gamepadActions": [
            "gamepadA", "gamepadB", "gamepadX", "gamepadY", "menu", "view", "guide",
            "leftBumper", "rightBumper", "leftThumb", "rightThumb", "dPadUp", "dPadDown",
            "dPadLeft", "dPadRight", "leftTrigger", "rightTrigger", "leftJoystickUp",
            "leftJoystickDown", "leftJoystickLeft", "leftJoystickRight", "rightJoystickUp",
            "rightJoystickDown", "rightJoystickLeft", "rightJoystickRight",
        ],
        "axisInputs": ["axisXY", "axisX", "axisY", "axisUp", "axisDown", "axisLeft", "axisRight"],
        "axisOutputs": [
            "leftJoystick", "rightJoystick", "leftJoystickX", "leftJoystickY", "rightJoystickX",
            "rightJoystickY", "leftTrigger", "rightTrigger", "leftJoystickUp", "leftJoystickDown",
            "leftJoystickLeft", "leftJoystickRight", "rightJoystickUp", "rightJoystickDown",
            "rightJoystickLeft", "rightJoystickRight",
        ],
        "versions": {
            "legacyV1": ["1", "1.0", "1.1", "1.2"], "legacyV2": ["2", "2.0"],
            "identity": ["latest", "3.0", "3.1", "3.2", "3.3", "3.4", "3.4.1", "3.5", "4.0", "4.1"],
        },
        "runtimeRules": inventory["runtimeRules"] + [{
            "id": "input.numeric-domains",
            "rule": "ActionThreshold 为非负有限值且缺省为0；DeadzoneThreshold 保留带符号有限值；Sensitivity 为正有限值。输出过滤非有限贡献并按手柄范围限幅。",
            "source": "docs/references/tak-babylon-touch-engine.md",
            "anchor": "6. actionThreshold / 7. deadzone 与 sensitivity",
            "schemaDefinitions": ["ActionThreshold", "DeadzoneThreshold", "Sensitivity"],
        }],
        "sources": {"layoutSchema": layout["sha256"], "iconAtlas": atlas_source["sha256"]},
    }
    encoded = json.dumps(contract, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
    contract["revision"] = "tak-v4.1-" + hashlib.sha256(encoded).hexdigest()[:16]
    output = REPO / "crates/xbox-touch-layouts/protocol"
    output.mkdir(parents=True, exist_ok=True)
    (output / "contract.json").write_text(json.dumps(contract, ensure_ascii=False, indent=2) + "\n")
    resources = REPO / "iosapp/XBXRC/Resources/TouchLayouts"
    (resources / "tak-icons.png").write_bytes(image)
    (resources / "tak-icons.json").write_text(json.dumps({
        "revision": contract["revision"], "dpi": 3, "frames": atlas["frames"],
        "dimensions": atlas["meta"]["dimensions"], "imageSHA256": hashlib.sha256(image).hexdigest(),
    }, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"revision": contract["revision"], "icons": len(atlas["frames"]),
                      "schemaDefinitions": len(layout["contract"]["$defs"]), "imageBytes": len(image)}))


if __name__ == "__main__":
    main()
