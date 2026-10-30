"""准备 Rust 审查副本，打印复现命令；生产源码保持原状。"""
import json
import pathlib
import shlex
import shutil
import tempfile
import tomllib

root = pathlib.Path(__file__).resolve().parents[2]
temporary = pathlib.Path(tempfile.mkdtemp(prefix="xbxrc-rust-startup-audit-"))
crate = root / "crates/xbox-streaming"
shutil.copytree(crate / "src", temporary / "src")
manifest = tomllib.loads((crate / "Cargo.toml").read_text())
workspace = tomllib.loads((root / "Cargo.toml").read_text())
for section in ("package", "dependencies"):
    for name, value in list(manifest[section].items()):
        if isinstance(value, dict) and value.get("workspace"):
            manifest[section][name] = workspace["workspace"][section][name]
        elif isinstance(value, dict) and "path" in value:
            value["path"] = str((crate / value["path"]).resolve())
manifest["profile.dev"] = workspace["profile"]["dev"]


def toml(value):
    if isinstance(value, dict):
        return "{ " + ", ".join(
            f"{json.dumps(key)} = {toml(item)}" for key, item in value.items()
        ) + " }"
    return json.dumps(value)


(temporary / "Cargo.toml").write_text("\n\n".join(
    "[" + name + "]\n" + "\n".join(
        f"{key} = {toml(value)}" for key, value in fields.items()
    ) for name, fields in manifest.items()
) + "\n")
shutil.copyfile(root / "Cargo.lock", temporary / "Cargo.lock")
tests = temporary / "src/session/flow.test.rs"
# 冷启动 HTTP 客户端需要初始化；时限调整只作用于临时测试夹具。
source = tests.read_text().replace(
    "plan.session.schedule.offer_exchange_timeout_ms = 120;",
    "plan.session.schedule.offer_exchange_timeout_ms = 5000;",
).replace(
    "plan.session.schedule.ice_exchange_timeout_ms = 120;",
    "plan.session.schedule.ice_exchange_timeout_ms = 5000;",
)
tests.write_text(source + "\n" + (
    root / "dev-docs/validation/2026-10-30-common-startup-audit.rs"
).read_text())
manifest_path = str(temporary / "Cargo.toml")
print(shlex.join(["cargo", "fmt", "--manifest-path", manifest_path]))
print("NO_PROXY=127.0.0.1,localhost no_proxy=127.0.0.1,localhost " + shlex.join([
    "cargo", "test", "--manifest-path", manifest_path, "--offline",
    "--target-dir", str(root / "target"), "--lib", "audit_common_startup",
    "--", "--nocapture",
]))
