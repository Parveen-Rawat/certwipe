#!/usr/bin/env bash

set -euo pipefail

echo "== Basic OS info =="

cat /etc/os-release || true
uname -a

echo 
echo "== Rust Toolchain =="
rustc --version || echo "rustc not found"
cargo --version || echo "cargo not found"
rustup --version || echo "rustup not found"
echo "Installed rustup toolchain:"
rustup show || true

echo 
echo "== useful system commands =="
echo -n "lsblk: "; which lsblk && lsblk --version || echo "lsblk missing"
echo -n "losetup: "; which losetip && losetup --help >/dev/null 2>&1 || echo "losetup missing"
echo -n "hdparm: "; which hdparm || echo "hdparm missing"
echo -n "nvme CLI: "; which nvme || echo "nvme CLI missing"
echo -n "smartctl: "; which smartctl || echo "smartctl missing"
echo -n "qemu-img: "; which qemu-img || echo "qemu-utils missing"
echo -n "imagemagick: "; (which magick || which convert) || echo "ImageMagick missing"
echo -n "chromium: "; which chromium || which chromium-browser || which google-chrome || echo "Chromium not found"

echo 
echo "==Python / wkhtmltopdf=="
which wkhtmltopdf && wkhtmltodf --version || echo "wkthmltopdf not installed"

echo 
echo "==Quick check done=="
echo "If anything above is missing, install via apt:"
echo "sudo apt update && sudo apt install -y build-essential pkg-config libssl-dev git curl util-linux hdparm nvme-cli qemu-utils smartmontools imagemagick chromium"

