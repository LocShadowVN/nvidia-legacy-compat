# nvidia-legacy-compat

> **Ngôn ngữ / Languages:** **Tiếng Việt** | **[English](README.md)**

Dự án mã nguồn mở cung cấp lớp tương thích (compatibility layer) và công cụ tự động hóa nhằm hồi sinh và duy trì các nhánh driver GPU NVIDIA độc quyền đã ngừng hỗ trợ (**470.xx**, **390.xx**, **340.xx**) trên các bản phân phối Linux hiện đại sử dụng **Linux Kernel 6.x+** và các **Wayland Compositor** thế hệ mới (Hyprland/wlroots, GNOME/Mutter, KDE/KWin).

---

## Mục lục
- [1. Tổng quan & Động lực phát triển](#1-tổng-quan--động-lực-phát-triển)
- [2. Kiến trúc kỹ thuật](#2-kiến-trúc-kỹ-thuật)
- [3. Ma trận hỗ trợ Driver & Kernel](#3-ma-trận-hỗ-trợ-driver--kernel)
- [4. Cấu trúc thư mục](#4-cấu-trúc-thư-mục)
- [5. Yêu cầu hệ thống](#5-yêu-cầu-hệ-thống)
- [6. Hướng dẫn cài đặt & Sử dụng](#6-hướng-dẫn-cài-đặt--sử-dụng)
- [7. Cấu hình cho Wayland Compositor](#7-cấu-hình-cho-wayland-compositor)
- [8. Kiểm thử tự động & Phát triển](#8-kiểm-thử-tự-động--phát-triển)
- [9. Giấy phép](#9-giấy-phép)

---

## 1. Tổng quan & Động lực phát triển

Các dòng GPU NVIDIA thế hệ cũ (kiến trúc Kepler, Fermi, Tesla) phụ thuộc vào các nhánh driver độc quyền (`470.xx`, `390.xx`, `340.xx`) đã bị NVIDIA ngừng cập nhật:
1. **Lỗi biên dịch trên Kernel mới:** Linux Kernel từ 6.3 đến 6.12+ thay đổi hàng loạt API nội bộ trong hệ thống quản lý bộ nhớ (`mm`) và Direct Rendering Manager (`drm`), khiến kernel module của driver cũ không thể biên dịch.
2. **Không tương thích với Wayland hiện đại:** Các driver cũ được thiết kế xoay quanh giao thức `EGLStreams` đã bị khai tử và thiếu hỗ trợ đầy đủ cho Generic Buffer Management (`GBM`) cũng như cơ chế đồng bộ tường minh (`Explicit Sync` / `linux-drm-syncobj-v1`), gây ra lỗi không khởi động được hoặc nhấp nháy khung hình (flickering) nghiêm trọng.

`nvidia-legacy-compat` giải quyết triệt để cả hai vấn đề thông qua kiến trúc 3 module mà không cần dịch ngược hay sửa đổi các file nhị phân đóng (closed-source blobs).

---

## 2. Kiến trúc kỹ thuật

### Module 1: Kernel Compatibility Patches (C / Kbuild)
Can thiệp vào lớp mã nguồn mở trung gian (`kernel/nvidia/`, `kernel/nvidia-drm/`, `kernel/nvidia-uvm/`) và tích hợp header `nv-linux-6x-compat.h`:
- **Linux 6.3+:** Thay thế các thao tác ghi trực tiếp vào `vma->vm_flags` bằng `vm_flags_set()` và `vm_flags_clear()`.
- **Linux 6.5+:** Cập nhật chữ ký hàm `get_user_pages()`, `pin_user_pages()` và `get_user_pages_remote()` sau khi Kernel loại bỏ tham số `struct vm_area_struct **vmas`.
- **Linux 6.8+:** Định nghĩa hằng số tương thích cho cờ `DRM_UNLOCKED` đã bị gỡ bỏ.
- **Linux 6.10+ & 6.12+:** Thay thế hàm `follow_pfn()` đã bị xóa bằng `follow_pte()` (trên Kernel 6.10–6.11) và `follow_pfnmap_start()`/`follow_pfnmap_end()` (trên Kernel 6.12+).
- **DRM PRIME & Atomic KMS:** Kích hoạt `DRIVER_MODESET | DRIVER_ATOMIC` và kết nối `DRM_IOCTL_PRIME_HANDLE_TO_FD` (`drm_gem_prime_handle_to_fd`) để xuất DMA-BUF.

### Module 2: Wayland GBM Translation Shim (Rust `cdylib`)
Biên dịch thành thư viện động `libgbm.so.1` thay thế với đầy đủ chuẩn C-ABI `GBM_1.0` của Mesa:
- **Chiến lược cấp phát Buffer:** Sử dụng `DRM_IOCTL_MODE_CREATE_DUMB` / GEM để cấp phát bộ nhớ từ DRM node và chuyển đổi GEM handle sang DMA-BUF File Descriptor qua `DRM_IOCTL_PRIME_HANDLE_TO_FD` để bàn giao cho Wayland Compositor.
- **Cơ chế đồng bộ chống nhấp nháy (`dma_fence` / `sync_file`):** Tích hợp `DMA_BUF_IOCTL_EXPORT_SYNC_FILE`, `DMA_BUF_IOCTL_IMPORT_SYNC_FILE` kết hợp `poll()` trên hàng rào đồng bộ của kernel nhằm ngăn chặn triệt để lỗi out-of-order rendering.

### Module 3: Automation & DKMS Pipeline (Bash / JSON)
- **`patches/patch-matrix.json`:** Ma trận định nghĩa điều kiện áp dụng từng bản vá `.patch` dựa trên phiên bản driver và dải phiên bản kernel (`min_kernel`, `max_kernel`).
- **`scripts/auto-patch.sh`:** Tự động giải nén gói cài đặt `.run`, phân tích SemVer của kernel hiện hành, kiểm tra thử nghiệm (`--dry-run`), áp dụng bản vá, cấu hình `nvidia-drm.modeset=1`, và đăng ký/biên dịch tự động với DKMS.

---

## 3. Ma trận hỗ trợ Driver & Kernel

| Nhánh Driver | Kiến trúc GPU mục tiêu | Kernel tối thiểu | Kernel tối đa đã kiểm thử | Hỗ trợ DRM PRIME / Wayland Shim |
| :--- | :--- | :---: | :---: | :---: |
| **470.256.02** | Kepler (GeForce GTX 600 / 700 series) | `6.0.0` | `6.12+` | Hỗ trợ đầy đủ |
| **390.157** | Fermi (GeForce GTX 400 / 500 series) | `6.0.0` | `6.12+` | Hỗ trợ đầy đủ |
| **340.108** | Tesla (GeForce 8000 / 9000 / 200 / 300) | `6.0.0` | `6.12.x` | Legacy KMS / X11 |

---

## 4. Cấu trúc thư mục

```text
nvidia-legacy-compat/
├── kernel-compat/
│   └── include/
│       └── nv-linux-6x-compat.h                  # Header C tương thích tập trung cho Kernel 6.x+
├── patches/
│   ├── patch-matrix.json                         # Ma trận ánh xạ bản vá theo Kernel & Driver
│   └── 470.256.02/
│       ├── 0001-kernel-6.3-vm-flags.patch
│       ├── 0002-kernel-6.5-gup-signature.patch
│       └── 0003-kernel-6.10-follow-pfn-drm.patch
├── gbm-shim/
│   ├── Cargo.toml                                # Cấu hình crate Rust cdylib
│   ├── linker.lds                                # Script định danh phiên bản symbol (GBM_1.0)
│   └── src/
│       ├── lib.rs                                # Triển khai các hàm C-ABI của libgbm.so
│       ├── drm_ioctl.rs                          # Các hàm gọi DRM Dumb / GEM / PRIME IOCTL
│       └── sync.rs                               # Bộ đồng bộ hóa DMA-BUF sync_file / dma_fence
├── scripts/
│   └── auto-patch.sh                             # Script tự động giải nén, patch và cài đặt DKMS
├── tests/
│   └── test_auto_patch.sh                        # Bộ kiểm thử tích hợp tự động
└── .github/workflows/ci.yml                      # Cấu hình GitHub Actions CI
```

---

## 5. Yêu cầu hệ thống

- **Công cụ hệ thống:** `bash`, `jq`, `patch`, `dkms`, `make`, `gcc`, `linux-headers-$(uname -r)`
- **Trình biên dịch Rust:** `rustc` và `cargo` (phiên bản 1.77+ stable) để biên dịch `gbm-shim`
- **Công cụ kiểm tra mã nguồn (Tùy chọn):** `shellcheck`, `clippy`, `rustfmt`

Trên Debian/Ubuntu:
```bash
sudo apt-get update
sudo apt-get install -y build-essential dkms linux-headers-$(uname -r) jq patch shellcheck cargo rustc
```

Trên Arch Linux:
```bash
sudo pacman -S --needed base-devel dkms linux-headers jq patch shellcheck rust
```

---

## 6. Hướng dẫn cài đặt & Sử dụng

### Bước 1: Vá mã nguồn và cài đặt Kernel Module qua DKMS

Tải gói cài đặt `.run` chính thức từ kho lưu trữ của NVIDIA, sau đó chạy `auto-patch.sh`:

```bash
chmod +x scripts/auto-patch.sh

# Tùy chọn: Kiểm tra trước khả năng tương thích của các bản vá ở chế độ dry-run
./scripts/auto-patch.sh \
  --driver-version 470.256.02 \
  --run-file ./NVIDIA-Linux-x86_64-470.256.02.run \
  --dry-run

# Áp dụng bản vá, kích hoạt nvidia-drm.modeset=1 và đăng ký/biên dịch với DKMS
sudo ./scripts/auto-patch.sh \
  --driver-version 470.256.02 \
  --run-file ./NVIDIA-Linux-x86_64-470.256.02.run
```

### Bước 2: Biên dịch và cài đặt Wayland GBM Shim (`libgbm.so.1`)

```bash
cd gbm-shim
RUSTFLAGS="-C link-arg=-Wl,--version-script=$(pwd)/linker.lds -C link-arg=-Wl,-soname,libgbm.so.1" \
  cargo build --release

sudo install -Dm755 target/release/libgbm.so /usr/local/lib/nvidia-legacy-gbm/libgbm.so.1
sudo ln -sf libgbm.so.1 /usr/local/lib/nvidia-legacy-gbm/libgbm.so
```

---

## 7. Cấu hình cho Wayland Compositor

Để khởi chạy Wayland Compositor sử dụng thư viện `nvidia-legacy-gbm` mà không cần ghi đè lên gói Mesa mặc định của hệ điều hành, hãy khai báo biến môi trường trong script khởi động phiên làm việc (session wrapper):

```bash
export LD_LIBRARY_PATH="/usr/local/lib/nvidia-legacy-gbm:${LD_LIBRARY_PATH:-}"
export GBM_BACKEND="nvidia-legacy-drm-gbm"
export WLR_NO_HARDWARE_CURSORS=1
```

Sau khi khởi động lại máy, kiểm tra trạng thái DRM KMS:
```bash
sudo cat /sys/module/nvidia_drm/parameters/modeset
# Kết quả mong đợi: Y
```

---

## 8. Kiểm thử tự động & Phát triển

### Chạy ShellCheck & Kiểm thử tích hợp Pipeline Patch
```bash
shellcheck scripts/auto-patch.sh tests/test_auto_patch.sh
chmod +x tests/test_auto_patch.sh
./tests/test_auto_patch.sh
```

### Chạy Unit Test cho Rust GBM Shim
Bộ kiểm thử Rust sử dụng `memfd_create` và `eventfd` để giả lập bộ nhớ DRM/PRIME và hàng rào đồng bộ `sync_file` ngay trên RAM, giúp kiểm thử toàn diện chuẩn C-ABI trên mọi máy trạm hoặc CI runner mà không cần lắp đặt card đồ họa NVIDIA vật lý:
```bash
cd gbm-shim
cargo test -- --nocapture
```

---

## 9. Giấy phép

Dự án này được phát hành dưới giấy phép **MIT License**. Lưu ý rằng các gói cài đặt driver NVIDIA gốc do người dùng tải về vẫn tuân theo thỏa thuận cấp phép phần mềm của NVIDIA (NVIDIA Software License Agreement).
