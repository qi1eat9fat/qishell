# 第三方声明

本项目自身以 GPL-3.0-only 发布（见 `LICENSE`）。下面这些第三方素材随仓库分发，各有自己的许可与要求。

## HarmonyOS Sans SC

- 用途：界面统一字体。`assets/fonts/` 下的 Regular / Medium / Bold 三个 TTF 是**华为发布的官方原文件，未做任何修改**。
- 版权：Copyright 2021 Huawei Device Co., Ltd.
- 许可：HarmonyOS Sans Fonts License Agreement，全文见 `assets/fonts/HARMONYOS-SANS-LICENSE.txt`。
- 许可要点：
  1. 必须在软件中显著声明使用了 HarmonyOS Sans 字体——本文即是该声明。
  2. **不得修改字体或其任何组件**。因此仓库里不能放子集化 / 改名过的版本；早期按字符集裁剪的 TTF 已全部换成原文件，`scripts/font_charset.py` 只生成字符清单用于核对覆盖范围，不裁剪字体。
  3. 不得将字体单独分发或销售；随本软件一起分发不受此限。

## Twemoji

- 用途：`assets/flags/` 下的 256 面国旗图片（ISO 3166-1 alpha-2 两位代码 → PNG）。
- 版权：Copyright 2020 Twitter, Inc and other contributors。
- 许可：CC-BY 4.0，<https://creativecommons.org/licenses/by/4.0/>。
- 项目主页：<https://github.com/twitter/twemoji>。

## 运行期依赖

Rust 依赖及其完整许可见 `Cargo.lock`。主要几项：

| crate | 许可 |
|---|---|
| gpui / gpui-kit 系列 | Apache-2.0 |
| alacritty_terminal | Apache-2.0 |
| russh / russh-sftp | Apache-2.0 |
| rusqlite | MIT |
| aes-gcm | MIT OR Apache-2.0 |
| tokio | MIT |

## 图标

`assets/qishell.ico` 与 `assets/qishell-icon.png` 为本项目原创，随项目以 GPL-3.0-only 发布。
