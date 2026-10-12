#!/usr/bin/env python3
"""从仓库根目录的 `icon.png` 生成管理后台所需的图标文件。

用法：
    python3 scripts/make-icons.py          # 或 make icons

依赖：Python 3.9+ 与 Pillow（`pip install pillow`）。

产物（写入 `web/public/`，Vite 会在构建时复制到 `dist/` 根目录）：

| 文件 | 用途 |
|---|---|
| `favicon.ico` | 16/32/48 多尺寸，浏览器根路径自动回退 |
| `favicon-16x16.png` / `favicon-32x32.png` | 现代浏览器显式声明的尺寸 |
| `apple-touch-icon.png` | iOS 主屏图标（180×180，扁平化到白色） |

处理约定（改动前请先读）：

1. **裁掉几乎不可见的光晕**：原图四周有一圈 alpha 很低的白色柔光（`alpha <= 16`）。
   保留它会在深色标签栏上糊成一团，也会让图标显得很小，因此按 `alpha > 16` 的包围盒裁剪。
2. **补成正方形**：浏览器会把 favicon 按正方形渲染，非正方形会被拉伸或留边；
   这里用**透明**补边并留 3% 内边距，保证图形不贴边。
   ⚠️ 不要改成白底扁平化——在深色标签栏上会变成一个白方块，比柔光更难看。
3. **apple-touch-icon 单独扁平化到白色**：iOS 不支持透明通道，不扁平化会被合成到黑色上。
4. 原图 `icon.png` 只用于 README 顶部展示，不参与这里的构建。
"""

from __future__ import annotations

import sys
from pathlib import Path

try:
    from PIL import Image
except ImportError:  # pragma: no cover
    sys.exit("缺少 Pillow，请先执行：pip install pillow")

REPO_ROOT = Path(__file__).resolve().parent.parent
SOURCE = REPO_ROOT / "icon.png"
OUT_DIR = REPO_ROOT / "web" / "public"

# 低于该 alpha 的像素视为柔光，裁剪时忽略
HALO_ALPHA_THRESHOLD = 16
# 正方形画布的额外边距比例
MARGIN_RATIO = 0.06
APPLE_SIZE = 180


def main() -> int:
    if not SOURCE.is_file():
        return fail(f"找不到源图：{SOURCE}")

    src = Image.open(SOURCE).convert("RGBA")
    content = src.getchannel("A").point(lambda v: 255 if v > HALO_ALPHA_THRESHOLD else 0).getbbox()
    if content is None:
        return fail(f"{SOURCE.name} 看起来是全透明的，无法生成图标")

    trimmed = src.crop(content)
    side = round(max(trimmed.size) * (1 + MARGIN_RATIO))
    canvas = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    canvas.paste(
        trimmed,
        ((side - trimmed.width) // 2, (side - trimmed.height) // 2),
        trimmed,
    )
    print(f"源图 {src.size[0]}×{src.size[1]} → 裁掉柔光后 {trimmed.size[0]}×{trimmed.size[1]} → 正方形画布 {side}×{side}")

    def scaled(size: int) -> Image.Image:
        return canvas.resize((size, size), Image.LANCZOS)

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    outputs: list[tuple[str, int]] = []

    for name, size in (("favicon-16x16.png", 16), ("favicon-32x32.png", 32)):
        path = OUT_DIR / name
        scaled(size).save(path, optimize=True)
        outputs.append((name, path.stat().st_size))

    # 多尺寸 ICO：Pillow 会从同一张源图分别缩放
    ico = OUT_DIR / "favicon.ico"
    scaled(48).save(ico, sizes=[(16, 16), (32, 32), (48, 48)])
    outputs.append(("favicon.ico", ico.stat().st_size))

    # iOS 不接受透明通道 → 扁平化到白色
    apple = Image.new("RGB", (APPLE_SIZE, APPLE_SIZE), (255, 255, 255))
    apple_icon = scaled(APPLE_SIZE)
    apple.paste(apple_icon, (0, 0), apple_icon)
    apple_path = OUT_DIR / "apple-touch-icon.png"
    apple.save(apple_path, optimize=True)
    outputs.append(("apple-touch-icon.png", apple_path.stat().st_size))

    print(f"\n写入 {OUT_DIR.relative_to(REPO_ROOT)}/：")
    for name, size in outputs:
        print(f"  {name:<24} {size:>7,} B")
    total = sum(size for _, size in outputs)
    print(f"  {'合计':<22} {total:>7,} B")
    return 0


def fail(message: str) -> int:
    print(f"错误：{message}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
