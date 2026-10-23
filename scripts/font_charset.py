"""从原型与设计文档里扫出界面用到的字符，生成一份字符清单。

**只生成清单，不裁剪字体。** HarmonyOS Sans 的许可是「不得修改字体或其任何
组件」，子集化属于修改，所以仓库里的 TTF 必须是官方原文件。这个脚本产出的
`.charset.txt` 只用来核对：界面里所有字符是否都落在原字体的覆盖范围内。

用法：python scripts/font_charset.py
"""

from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]


def sources() -> list[Path]:
    """界面文本来源：源码 + 同目录的设计文档与原型。"""
    files = sorted((ROOT / "src").rglob("*.rs")) if (ROOT / "src").exists() else []
    # 上层目录是开发时的原型与交接文档；仓库里没有它时自动跳过。
    for name in [
        "index.html",
        "ssh-app-第一期设计说明与开发交接.md",
        "ssh-app-MVP范围.md",
        "ssh-app-UI页面清单.md",
    ]:
        candidate = ROOT.parent / name
        if candidate.exists():
            files.append(candidate)
    return files


def main() -> int:
    chars = set()
    used = []
    for path in sources():
        text = path.read_text(encoding="utf-8", errors="ignore")
        chars.update(text)
        used.append(path)

    ordered = "".join(sorted(chars))
    out = ROOT / "assets" / "fonts" / ".charset.txt"
    out.write_text(ordered, encoding="utf-8")

    print(f"扫了 {len(used)} 个文件，去重后 {len(chars)} 个字符")
    print(f"清单写入 {out.relative_to(ROOT)}")
    print("字体必须使用官方原文件，不要按这份清单裁剪。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
