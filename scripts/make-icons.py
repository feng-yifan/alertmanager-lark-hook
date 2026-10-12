#!/usr/bin/env python3
"""派生管理后台所需的图标文件。

用法：
    python3 scripts/make-icons.py          # 或 make icons

依赖：Python 3.9+ 与 Pillow（`pip install pillow`）。

## 源文件（均已入库，是本项目的品牌资产）

| 路径 | 说明 |
|---|---|
| `web/public/favicon.ico` | **品牌主图标**，直接入库、不经生成。浏览器取用，本脚本**只读不写** |
| `docs/images/banner.png` | 高清品牌图（1536×1024，深色底）。README 顶部展示，同时作为大尺寸图标的来源 |

## 产物（写入 `web/public/`，Vite 构建时复制到 `dist/` 根目录）

| 文件 | 来源 | 说明 |
|---|---|---|
| `favicon-16x16.png` | `favicon.ico` 降采样 | 标签页最小尺寸，浏览器可避免自行缩放 |
| `favicon-32x32.png` | `favicon.ico` 原尺寸 | 与主图标**逐像素同源**，保证一致 |
| `apple-touch-icon.png` | `banner.png` | 180×180。iOS 会自行加圆角遮罩，所以直接铺满、不做留白 |

## 为什么 apple-touch-icon 用另一张源图

`favicon.ico` 只有 32×32，放大到 180 会明显发虚。`banner.png` 是同一套视觉的高清版本，
方形化后缩到 180 边缘锐利，且深色底正好适合 iOS 主屏。

## 处理约定（改动前请先读）

1. **不要用 `favicon.ico` 生成 180px 图标**（见上）。
2. **方形化用边缘延展，而不是填纯色**：`banner.png` 的背景带轻微渐变，
   填纯色会留下可见接缝；这里把首行/末行像素向上下延展，接缝不可见，
   也不会像「居中裁方」那样把右侧卡片切掉。
3. `favicon-16x16.png` / `favicon-32x32.png` 必须与 `favicon.ico` 同源，
   否则不同尺寸下会出现两种视觉。
4. 本脚本不再从仓库根目录读取任何图片；根目录不再存放图片资源。
"""

from __future__ import annotations

import sys
from pathlib import Path

try:
    from PIL import Image
except ImportError:  # pragma: no cover
    sys.exit("缺少 Pillow，请先执行：pip install pillow")

REPO_ROOT = Path(__file__).resolve().parent.parent
OUT_DIR = REPO_ROOT / "web" / "public"
FAVICON = OUT_DIR / "favicon.ico"
BANNER = REPO_ROOT / "docs" / "images" / "banner.png"

APPLE_SIZE = 180
PNG_SIZES = (("favicon-16x16.png", 16), ("favicon-32x32.png", 32))


def pad_to_square_by_edge(img: Image.Image) -> Image.Image:
    """把长方形补成正方形：用首行/末行（或首列/末列）向外延展，避免接缝与裁切。"""
    w, h = img.size
    if w == h:
        return img

    side = max(w, h)
    canvas = Image.new(img.mode, (side, side))
    if w > h:  # 上下补
        pad = (side - h) // 2
        top = img.crop((0, 0, w, 1)).resize((w, pad), Image.NEAREST)
        bottom = img.crop((0, h - 1, w, h)).resize((w, side - h - pad), Image.NEAREST)
        canvas.paste(top, (0, 0))
        canvas.paste(img, (0, pad))
        canvas.paste(bottom, (0, pad + h))
    else:  # 左右补
        pad = (side - w) // 2
        left = img.crop((0, 0, 1, h)).resize((pad, h), Image.NEAREST)
        right = img.crop((w - 1, 0, w, h)).resize((side - w - pad, h), Image.NEAREST)
        canvas.paste(left, (0, 0))
        canvas.paste(img, (pad, 0))
        canvas.paste(right, (pad + w, 0))
    return canvas


def main() -> int:
    if not FAVICON.is_file():
        return fail(f"找不到品牌主图标：{FAVICON.relative_to(REPO_ROOT)}")
    if not BANNER.is_file():
        return fail(f"找不到高清品牌图：{BANNER.relative_to(REPO_ROOT)}")

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    ico = Image.open(FAVICON).convert("RGBA")
    sizes = sorted(ico.ico.sizes()) if hasattr(ico, "ico") and ico.ico else [ico.size]
    print(f"主图标 {FAVICON.relative_to(REPO_ROOT)}：{ico.size[0]}×{ico.size[1]}，内含尺寸 {sizes}")

    outputs: list[tuple[str, int]] = []

    # 1) 与主图标同源的小尺寸 PNG
    for name, size in PNG_SIZES:
        path = OUT_DIR / name
        ico.resize((size, size), Image.LANCZOS).save(path, optimize=True)
        outputs.append((name, path.stat().st_size))

    # 2) apple-touch-icon：高清图方形化后缩到 180
    banner = Image.open(BANNER).convert("RGBA")
    squared = pad_to_square_by_edge(banner)
    apple = Image.new("RGB", (APPLE_SIZE, APPLE_SIZE), (0, 0, 0))
    apple_icon = squared.resize((APPLE_SIZE, APPLE_SIZE), Image.LANCZOS).convert("RGBA")
    apple.paste(apple_icon, (0, 0), apple_icon)
    apple_path = OUT_DIR / "apple-touch-icon.png"
    apple.save(apple_path, optimize=True)
    outputs.append(("apple-touch-icon.png", apple_path.stat().st_size))
    print(
        f"高清图 {BANNER.relative_to(REPO_ROOT)}：{banner.size[0]}×{banner.size[1]} "
        f"→ 方形化 {squared.size[0]}×{squared.size[1]} → {APPLE_SIZE}×{APPLE_SIZE}"
    )

    print(f"\n写入 {OUT_DIR.relative_to(REPO_ROOT)}/（favicon.ico 原样保留，未改动）：")
    for name, size in outputs:
        print(f"  {name:<24} {size:>7,} B")
    print(f"  {'合计':<22} {sum(s for _, s in outputs):>7,} B")
    return 0


def fail(message: str) -> int:
    print(f"错误：{message}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
