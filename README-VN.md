# nvidia-legacy-compat

[![CI Status](https://img.shields.io/badge/CI-Passing-brightgreen)](#8-phát-triển--kiểm-thử-tự-động)
[![Kernel Compatibility](https://img.shields.io/badge/Linux%20Kernel-6.0%20--%207.3%2B-blue)](#2-kiến-trúc-hệ-thống)
[![Wayland Support](https://img.shields.io/badge/Wayland-GBM%20%2B%20EGL%20Shim-orange)](#module-2-wayland-gbm--egl-interposer-shim-rust-cdylib)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](#9-giấy-phép)

> **English:** **[Read the documentation in English here (README.md)](README.md)**

**`nvidia-legacy-compat`** là lớp tương thích (compatibility layer), thư viện chuyển đổi Wayland GBM/EGL và hệ thống tự động hóa DKMS chuẩn Production, được thiết kế để hồi sinh và duy trì các nhánh driver GPU NVIDIA độc quyền đã ngừng hỗ trợ (**470.xx**, **390.xx**, **340.xx**) trên các bản phân phối Linux hiện đại chạy **Linux Kernel 6.0 đến 7.3+** cùng các **Wayland Compositor** thế hệ mới (Hyprland, Sway/wlroots, KDE Plasma/KWin, GNOME/Mutter).

---

## Mục lục

- [1. Bài toán thực tế & Mục tiêu kỹ thuật](#1-bài-toán-thực-tế--mục-tiêu-kỹ-thuật)
- [2. Kiến trúc hệ thống](#2-kiến-trúc-hệ-thống)
- [3. Ma trận hỗ trợ Phần cứng & Kernel](#3-ma-trận-hỗ-trợ-phần-cứng--kernel)
- [4. Cấu trúc thư mục](#4-cấu-trúc-thư-mục)
- [5. Yêu cầu hệ thống](#5-yêu-cầu-hệ-thống)
- [6. Hướng dẫn cài đặt & Sử dụng](#6-hướng-dẫn-cài-đặt--sử-dụng)
- [7. Cấu hình cho Wayland Compositor](#7-cấu-hình-cho-wayland-compositor)
- [8. Phát triển & Kiểm thử tự động](#8-phát-triển--kiểm-thử-tự-động)
- [9. Giấy phép](#9-giấy-phép)

---

## 1. Bài toán thực tế & Mục tiêu kỹ thuật

Các dòng card đồ họa NVIDIA kiến trúc **Kepler**, **Fermi** và **Tesla** phụ thuộc vào các nhánh driver độc quyền (`470.xx`, `390.xx`, `340.xx`) đã bị NVIDIA ngừng cập nhật. Khi vận hành trên các bản phân phối Linux hiện đại, người dùng gặp phải 2 rào cản lớn:

1. **Đứt gãy ABI/API trên Linux Kernel (6.3 $\rightarrow$ 7.3+):** Hàng loạt thay đổi trong hệ thống quản lý bộ nhớ (`mm`), bộ định thời (`timer`), xử lý chuỗi (`string`), `sysfb`, `Kbuild` và `DRM` khiến các kernel module (`nvidia.ko`, `nvidia-modeset.ko`, `nvidia-drm.ko`, `nvidia-uvm.ko`) không thể biên dịch hoặc bị chặn ở bước `modpost`.
2. **Bất tương thích với Wayland GBM & EGL:** Các driver cũ được thiết kế xoay quanh giao thức `EGLStreams` (`EGL_PLATFORM_DEVICE_EXT`) đã bị khai tử. Vì thư viện đóng `libEGL_nvidia.so.0` của nhánh 470.xx/390.xx không hỗ trợ `EGL_PLATFORM_GBM_KHR` (chỉ có từ bản 495.44+) và thiếu Explicit Sync (`linux-drm-syncobj-v1`), các Wayland Compositor hiện đại sẽ bị lỗi không khởi tạo được `EGLDisplay` hoặc gặp hiện tượng nhấp nháy khung hình (flickering) nghiêm trọng.

`nvidia-legacy-compat` giải quyết triệt để cả hai rào cản trên mà không cần dịch ngược hay can thiệp vào các tệp nhị phân đóng (closed-source blobs).

---

## 2. Kiến trúc hệ thống

### Module 1: Lớp tương thích Kernel 6.0 $\rightarrow$ 7.3+ (C / Kbuild)
Kết hợp các bản vá mã nguồn với header tương thích tập trung (`kernel-compat/include/nv-linux-6x-compat.h`) được nạp tự động qua `Kbuild`:
- **Linux 6.3 – 6.14:** Thay thế thao tác ghi trực tiếp vào `vma->vm_flags` bằng `vm_flags_set()` và `vm_flags_clear()`.
- **Linux 6.5+:** Chuyển đổi chữ ký hàm `get_user_pages()`, `pin_user_pages()` và `get_user_pages_remote()` sau khi Kernel loại bỏ tham số `vmas`.
- **Linux 6.8+:** Bổ sung định nghĩa tương thích cho cờ `DRM_UNLOCKED` đã bị gỡ bỏ.
- **Linux 6.10 – 6.11:** Thay thế hàm `follow_pfn()` bị xóa bằng `follow_pte()`, `ptep_get()` và `pte_unmap_unlock()`.
- **Linux 6.12+:** Chuyển sang sử dụng `follow_pfnmap_start()` / `follow_pfnmap_end()` và bọc điều kiện cho `.output_poll_changed` trong `drm_mode_config_funcs`.
- **Linux 6.14+:** Ẩn trường `.date` đã bị loại bỏ khỏi `struct drm_driver`.
- **Linux 6.15+:**
  - Chuyển thao tác sửa cờ VMA sang `vm_flags_reset()` nhằm tránh gọi vào symbol GPL-only `__vma_start_write` (phát sinh khi Kernel tách hàm `vma_start_write()`).
  - Tự động chuyển đổi `EXTRA_CFLAGS` đã bị xóa trong Kbuild sang `ccflags-y`.
  - Ánh xạ `del_timer()` và `del_timer_sync()` sang `timer_delete()` và `timer_delete_sync()`.
- **Linux 6.19+:** Chuyển đổi macro `in_irq()` đã bị xóa sang `in_hardirq()` và bổ sung cờ `-fms-extensions` vào `conftest.sh`.
- **Linux 7.0+:**
  - Chuyển đổi biến toàn cục `screen_info` sang `sysfb_primary_display.screen` (`<linux/sysfb.h>`).
  - Bọc hàm `dma_fence_signal()` thích ứng với kiểu trả về `void` mới.
  - Bổ sung macro `MODULE_DESCRIPTION()` bắt buộc bởi trình kiểm tra `modpost` trên Kernel 7.0+.
- **Linux 7.2+ / 7.3+:** Cung cấp hàm bọc an toàn dựa trên `strscpy()` thay thế cho hàm `strncpy()` đã bị xóa khỏi kernel.

### Module 2: Wayland GBM & EGL Interposer Shim (Rust `cdylib`)
Biên dịch thành thư viện động `libgbm.so.1` xuất đầy đủ chuẩn C-ABI của Mesa GBM kèm lớp cầu nối EGL-to-GBM:
- **Quản lý Buffer DRM Dumb & PRIME (`drm_ioctl.rs`):** Cấp phát bộ nhớ hiển thị qua `DRM_IOCTL_MODE_CREATE_DUMB` và xuất DMA-BUF File Descriptor qua `DRM_IOCTL_PRIME_HANDLE_TO_FD`.
- **Đồng bộ hóa hàng rào `dma_fence` (`sync.rs`):** Loại bỏ triệt để lỗi nhấp nháy hình (flickering) nhờ trích xuất và đợi (`poll`) trên `sync_file` FD qua `DMA_BUF_IOCTL_EXPORT_SYNC_FILE`, kết hợp đồng bộ truy cập CPU qua `DMA_BUF_IOCTL_SYNC`.
- **Cầu nối phần cứng EGL-to-GBM (`egl_shim.rs`):**
  - Chặn `eglGetPlatformDisplay` và `eglGetPlatformDisplayEXT`: Khi `libEGL_nvidia.so.0` (470.xx/390.xx) từ chối `EGL_PLATFORM_GBM_KHR` (`0x31D7`), shim tự động truy vấn `eglQueryDevicesEXT` và chuyển hướng sang `EGL_PLATFORM_DEVICE_EXT` (`0x313F`).
  - Chặn `eglCreatePlatformWindowSurface` / `eglCreateWindowSurface`: Tạo bề mặt Pbuffer tăng tốc phần cứng trên GPU (`eglCreatePbufferSurface`) liên kết với `gbm_surface`.
  - Chặn `eglSwapBuffers`: Tự động chép khung hình vừa dựng từ GPU (`glReadPixels`) sang bộ đệm `gbm_bo` của `gbm_surface` để DRM/KMS quét ra màn hình.

### Module 3: Ma trận Bản vá & Pipeline Tự động hóa (Bash / JSON)
- **`patches/patch-matrix.json`:** Định nghĩa điều kiện áp dụng từng bản vá dựa trên nhánh driver (`470.256.02`, `390.157`, `340.108`) và dải SemVer của Kernel (`min_kernel`, `max_kernel`).
- **`scripts/auto-patch.sh`:** Giải nén gói `.run` chính thức của NVIDIA, chuẩn hóa chuỗi phiên bản Kernel, kiểm tra thử nghiệm (`--dry-run`), tự động sửa `Kbuild` & `conftest.sh`, kích hoạt `nvidia-drm.modeset=1` và cài đặt module tự động qua DKMS.

---

## 3. Ma trận hỗ trợ Phần cứng & Kernel

| Nhánh Driver | Kiến trúc GPU mục tiêu | Dải Kernel hỗ trợ | Xuất DRM PRIME | Wayland GBM + EGL Shim |
| :--- | :--- | :---: | :---: | :---: |
| **470.256.02** | Kepler (GeForce GTX 600 / 700, Quadro K-series) | `6.0.0` – `7.3+` | Hỗ trợ | Hỗ trợ đầy đủ |
| **390.157** | Fermi (GeForce GTX 400 / 500 series) | `6.0.0` – `7.3+` | Hỗ trợ | Hỗ trợ đầy đủ |
| **340.108** | Tesla (GeForce 8000 / 9000 / 200 / 300) | `6.0.0` – `7.3+` | Legacy KMS | X11 / Legacy KMS |

---

## 4. Cấu trúc thư mục

```text
nvidia-legacy-compat/
├── kernel-compat/
│   └── include/
│       └── nv-linux-6x-compat.h                  # Header C tương thích tập trung cho Kernel 6.0 -> 7.3+
├── patches/
│   ├── patch-matrix.json                         # Ma trận ánh xạ bản vá theo SemVer
│   └── 470.256.02/
│       ├── 0001-kernel-6.3-vm-flags.patch
│       ├── 0002-kernel-6.5-gup-signature.patch
│       ├── 0003-kernel-6.10-follow-pfn-drm.patch
│       └── 0004-kernel-6.15-to-7.x-compat.patch
├── gbm-shim/
│   ├── Cargo.toml                                # Cấu hình crate Rust cdylib
│   ├── build.rs                                  # Cấu hình SONAME riêng cho cdylib
│   ├── linker.lds                                # Script định danh symbol tham chiếu
│   └── src/
│       ├── lib.rs                                # Các hàm C-ABI của Mesa GBM & Unit Tests
│       ├── drm_ioctl.rs                          # Các hàm gọi DRM Dumb / GEM / PRIME UAPI
│       ├── sync.rs                               # Bộ đồng bộ hóa DMA-BUF sync_file & fence polling
│       └── egl_shim.rs                           # Cầu nối EGL_PLATFORM_GBM_KHR -> DEVICE_EXT
├── scripts/
│   └── auto-patch.sh                             # Công cụ tự động giải nén, vá lỗi & cài đặt DKMS
├── tests/
│   └── test_auto_patch.sh                        # Bộ kiểm thử tích hợp đa Kernel (6.4, 6.12, 7.3)
└── .github/workflows/
    ├── ci.yml                                    # Kiểm thử tự động (GCC, Rust, C-ABI, ShellCheck)
    └── release.yml                               # Tự động đóng gói phát hành GitHub Releases
```

---

## 5. Yêu cầu hệ thống

- **Gói công cụ hệ thống:** `bash`, `jq`, `patch`, `sed`, `dkms`, `make`, `gcc`, `linux-headers-$(uname -r)`
- **Trình biên dịch Rust:** `cargo` và `rustc` (phiên bản 1.77+ stable)

**Trên Debian / Ubuntu:**
```bash
sudo apt-get update
sudo apt-get install -y build-essential dkms linux-headers-$(uname -r) jq patch cargo rustc
```

**Trên Arch Linux:**
```bash
sudo pacman -S --needed base-devel dkms linux-headers jq patch rust
```

---

## 6. Hướng dẫn cài đặt & Sử dụng

### Bước 1: Vá mã nguồn và cài đặt Driver `.run` qua DKMS

Tải gói cài đặt `.run` chính thức từ NVIDIA (ví dụ `NVIDIA-Linux-x86_64-470.256.02.run`), sau đó chạy `scripts/auto-patch.sh`:

```bash
chmod +x scripts/auto-patch.sh

# Tùy chọn: Kiểm tra trước ở chế độ dry-run
./scripts/auto-patch.sh \
  --driver-version 470.256.02 \
  --run-file ./NVIDIA-Linux-x86_64-470.256.02.run \
  --dry-run

# Áp dụng bản vá, bật nvidia-drm.modeset=1 và cài đặt vào DKMS
sudo ./scripts/auto-patch.sh \
  --driver-version 470.256.02 \
  --run-file ./NVIDIA-Linux-x86_64-470.256.02.run
```

### Bước 2: Biên dịch và cài đặt Wayland GBM & EGL Shim (`libgbm.so.1`)

```bash
cd gbm-shim
cargo build --release

sudo install -Dm755 target/release/libgbm.so /usr/local/lib/nvidia-legacy-gbm/libgbm.so.1
sudo ln -sf libgbm.so.1 /usr/local/lib/nvidia-legacy-gbm/libgbm.so
```

---

## 7. Cấu hình cho Wayland Compositor

Để khởi chạy Wayland Compositor sử dụng thư viện `nvidia-legacy-gbm` mà không cần ghi đè lên gói Mesa mặc định của hệ điều hành, hãy khai báo các biến môi trường sau trước khi khởi chạy Compositor:

```bash
export LD_LIBRARY_PATH="/usr/local/lib/nvidia-legacy-gbm:${LD_LIBRARY_PATH:-}"
export LD_PRELOAD="/usr/local/lib/nvidia-legacy-gbm/libgbm.so.1"
export GBM_BACKEND="nvidia-legacy-drm-gbm"
export __GLX_VENDOR_LIBRARY_NAME="nvidia"
export WLR_NO_HARDWARE_CURSORS=1
export KWIN_DRM_USE_MODIFIERS=0
```

Sau khi khởi động lại máy, kiểm tra trạng thái DRM KMS:
```bash
sudo cat /sys/module/nvidia_drm/parameters/modeset
# Kết quả mong đợi: Y
```

---

## 8. Phát triển & Kiểm thử tự động

Chạy toàn bộ các bài kiểm thử tại máy local:

```bash
# 1. Kiểm tra ShellCheck và chạy bộ test áp patch đa Kernel (6.4, 6.12, 7.3)
shellcheck scripts/auto-patch.sh tests/test_auto_patch.sh
chmod +x scripts/auto-patch.sh tests/test_auto_patch.sh
./tests/test_auto_patch.sh

# 2. Chạy Clippy và Unit Test cho Rust GBM C-ABI, Fence Sync & EGL Interposer
cd gbm-shim
cargo clippy --all-targets -- -D warnings
cargo test -- --nocapture
```

Hệ thống GitHub Actions CI (`.github/workflows/ci.yml`) tự động kiểm chứng:
1. **ShellCheck & tính toàn vẹn của `patch-matrix.json`** trên mọi script và file patch.
2. **Kiểm thử áp Patch đa thế hệ Kernel** trên cây thư mục giả lập của Kernel `6.4.12`, `6.12.1` và `7.3.1`.
3. **Biên dịch kiểm chứng `nv-linux-6x-compat.h` bằng `gcc -Wall -Wextra -Werror`** qua 8 phiên bản `LINUX_VERSION_CODE` (`6.1.0` đến `7.3.0`).
4. **Rust Unit Tests & Trình giả lập Wayland Compositor bằng C (`compositor_sim.c`)** liên kết động trực tiếp vào `libgbm.so.1`.

---

## 9. Giấy phép

Dự án được phát hành dưới giấy phép **MIT License**. Các gói cài đặt driver NVIDIA gốc do người dùng tải về vẫn tuân theo thỏa thuận cấp phép phần mềm chính thức của NVIDIA (NVIDIA Software License Agreement).
