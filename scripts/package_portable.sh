#!/usr/bin/env bash
# 把 release 产物整理成一个可直接拷走的便携文件夹。
#
# 用法：scripts/package_portable.sh [目标目录]
# 默认输出到 <仓库>/dist/QiShell-portable。
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-$root/dist/QiShell-portable}"
src="${CARGO_TARGET_DIR:-$root/target}/release/qishell.exe"

if [ ! -f "$src" ]; then
    echo "找不到 $src，先跑 cargo build --release" >&2
    exit 1
fi

rm -rf "$out"
mkdir -p "$out"
cp "$src" "$out/qishell.exe"
cp "$root/LICENSE" "$out/LICENSE.txt"

# data\ 不放任何东西：应用第一次启动自己建，这样「拷走整个文件夹」不会
# 把开发机的数据库、密码、known_hosts 一起带出去。
echo "已生成 $(du -h "$out/qishell.exe" | cut -f1) 的 $out"
ls -la "$out"
