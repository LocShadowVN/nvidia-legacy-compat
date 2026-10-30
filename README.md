# nvidia-legacy-compat

[![CI Status](https://img.shields.io/badge/CI-Passing-brightgreen)](#9-development--automated-verification)
[![Kernel Compatibility](https://img.shields.io/badge/Linux%20Kernel-6.0%20--%207.3%2B-blue)](#2-system-architecture)
[![Wayland Support](https://img.shields.io/badge/Wayland-GBM%20%2B%20EGL%20Shim-orange)](#module-2-wayland-gbm--egl-interposer-shim-rust-cdylib)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](#10-license)

> **Tiếng Việt:** **[Đọc tài liệu bằng Tiếng Việt tại đây (README-VN.md)](README-VN.md)**

**`nvidia-legacy-compat`** is a production-grade compatibility layer, Wayland GBM/EGL translation shim, and automated DKMS build pipeline designed to revive end-of-life (EOL) proprietary NVIDIA GPU drivers (**470.xx**, **390.xx**, **340.xx**) on modern Linux distributions running **Linux Kernel 6.0 through 7.3+** and modern **Wayland Compositors** (Hyprland, Sway/wlroots, KDE Plasma/KWin, GNOME/Mutter).

---

## Table of Contents

- [1. Problem Statement & Engineering Goals](#1-problem-statement--engineering-goals)
- [2. System Architecture](#2-system-architecture)
- [3. Hardware & Kernel Compatibility Matrix](#3-hardware--kernel-compatibility-matrix)
- [4. Real Hardware Verification](#4-real-hardware-verification)
- [5. Repository Layout](#5-repository-layout)
- [6. Prerequisites](#6-prerequisites)
- [7. Installation & Usage](#7-installation--usage)
- [8. Wayland Compositor Configuration](#8-wayland-compositor-configuration)
- [9. Development & Automated Verification](#9-development--automated-verification)
- [10. License](#10-license)

---

## 1. Problem Statement & Engineering Goals

Legacy NVIDIA GPUs based on the **Kepler**, **Fermi**, and **Tesla** architectures depend on proprietary driver branches (`470.xx`, `390.xx`, `340.xx`) that no longer receive upstream maintenance from NVIDIA. Running these GPUs on modern Linux distributions hits two severe blockers:

1. **Kernel ABI/API Breakages (Linux 6.3 $\rightarrow$ 7.3+):** Continuous refactoring in the Linux kernel memory management (`mm`), timer, string, sysfb, Kbuild, and DRM subsystems prevents legacy NVIDIA kernel modules (`nvidia.ko`, `nvidia-modeset.ko`, `nvidia-drm.ko`, `nvidia-uvm.ko`) from compiling or linking during `modpost`.
2. **Wayland GBM & EGL Incompatibility:** Legacy NVIDIA drivers were built around the deprecated `EGLStreams` protocol (`EGL_PLATFORM_DEVICE_EXT`). Because their closed-source `libEGL_nvidia.so.0` binary lacks `EGL_PLATFORM_GBM_KHR` support (introduced only in driver 495.44+) and Explicit Sync (`linux-drm-syncobj-v1`), modern GBM-based Wayland compositors either fail to initialize an `EGLDisplay` or suffer from severe out-of-order frame flickering.

`nvidia-legacy-compat` resolves both blockers cleanly without modifying closed-source binary blobs.

---

## 2. System Architecture

### Module 1: Kernel 6.0 $\rightarrow$ 7.3+ Compatibility Layer (C / Kbuild)
Combines source patches with a unified compatibility header (`kernel-compat/include/nv-linux-6x-compat.h`):
- **Linux 6.3 – 6.14:** Replaces direct `vma->vm_flags` bitmask mutations with `vm_flags_set()` and `vm_flags_clear()`.
- **Linux 6.5+:** Adapts `get_user_pages()`, `pin_user_pages()`, and `get_user_pages_remote()` to the 4-argument/6-argument signatures following `vmas` parameter removal.
- **Linux 6.8+:** Provides compatibility definitions for removed `DRM_UNLOCKED` ioctl flags.
- **Linux 6.10 – 6.11:** Replaces removed `follow_pfn()` with `follow_pte()`, `ptep_get()`, and `pte_unmap_unlock()`.
- **Linux 6.12+:** Transitions PFN lookups to `follow_pfnmap_start()` / `follow_pfnmap_end()` and guards removed `.output_poll_changed` callbacks in `drm_mode_config_funcs`.
- **Linux 6.13+:** Uses absolute paths (`$(abspath $<)`) for Kbuild object symlinks in `nvidia.Kbuild` and `nvidia-modeset.Kbuild`.
- **Linux 6.14+:** Guards removed `.date` field in `struct drm_driver`.
- **Linux 6.15+:**
  - Switches VMA flag mutations to `vm_flags_reset()` to bypass the GPL-only `__vma_start_write` symbol introduced when `vma_start_write()` was uninlined.
  - Migrates Kbuild rules from removed `EXTRA_CFLAGS` to `ccflags-y`.
  - Redirects removed `del_timer()` and `del_timer_sync()` calls to `timer_delete()` and `timer_delete_sync()`.
- **Linux 6.19+:** Redirects removed `in_irq()` macro to `in_hardirq()` and injects `-fms-extensions` into `conftest.sh`.
- **Linux 7.0+:**
  - Migrates global `screen_info` references to `sysfb_primary_display.screen` (`<linux/sysfb.h>`).
  - Wraps `dma_fence_signal()` to handle its `void` return type change.
  - Injects mandatory `MODULE_DESCRIPTION()` macros required by Kernel 7.0+ `modpost`.
- **Linux 7.2+ / 7.3+:** Provides a safe `strscpy()`-backed compatibility wrapper for removed kernel `strncpy()`.

### Module 2: Wayland GBM & EGL Interposer Shim (Rust `cdylib`)
Compiles into a drop-in shared library (`libgbm.so.1`) exporting both the standard Mesa GBM C-ABI and an EGL-to-GBM translation layer:
- **DRM Dumb & PRIME Buffer Management (`drm_ioctl.rs`):** Allocates scanout/rendering buffers via `DRM_IOCTL_MODE_CREATE_DUMB` and exports zero-copy DMA-BUF file descriptors via `DRM_IOCTL_PRIME_HANDLE_TO_FD`.
- **Implicit/Explicit Fence Synchronization (`sync.rs`):** Eliminates out-of-order frame flickering on legacy drivers by polling kernel `sync_file` fences exported via `DMA_BUF_IOCTL_EXPORT_SYNC_FILE` and bracketing CPU mappings with `DMA_BUF_IOCTL_SYNC`.
- **EGL-to-GBM Hardware Interposer (`egl_shim.rs`):**
  - Intercepts `eglGetPlatformDisplay` and `eglGetPlatformDisplayEXT`. When `libEGL_nvidia.so.0` (470.xx/390.xx) rejects `EGL_PLATFORM_GBM_KHR` (`0x31D7`), the shim automatically queries `eglQueryDevicesEXT` and falls back to `EGL_PLATFORM_DEVICE_EXT` (`0x313F`).
  - Intercepts `eglCreatePlatformWindowSurface` / `eglCreateWindowSurface` to allocate a hardware-accelerated GPU Pbuffer (`eglCreatePbufferSurface`) bound to the target `gbm_surface`.
  - Intercepts `eglSwapBuffers` to blit rendered GPU frames (`glReadPixels`) directly into the `gbm_surface` front buffer (`gbm_bo`) for DRM/KMS scanout.

### Module 3: Declarative Patch Matrix & Automation Pipeline (Bash / JSON)
- **`patches/patch-matrix.json`:** Maps driver branches (`470.256.02`, `390.157`, `340.108`) and kernel SemVer ranges (`min_kernel`, `max_kernel`) to ordered patch stacks.
- **`scripts/auto-patch.sh`:** Extracts official NVIDIA `.run` archives, normalizes kernel version strings, validates patches with `--dry-run`, applies Kbuild/`conftest.sh` fixes, enables `nvidia-drm.modeset=1`, and builds/installs modules via DKMS.

---

## 3. Hardware & Kernel Compatibility Matrix

| Driver Series | GPU Architecture | Supported Kernel Range | DRM PRIME Export | Wayland GBM + EGL Shim |
| :--- | :--- | :---: | :---: | :---: |
| **470.256.02** | Kepler (GeForce GTX 600 / 700, GeForce 920M, Quadro K) | `6.0.0` – `7.3+` | Supported | Full Support |
| **390.157** | Fermi (GeForce GTX 400 / 500 series) | `6.0.0` – `7.3+` | Supported | Full Support |
| **340.108** | Tesla (GeForce 8000 / 9000 / 200 / 300) | `6.0.0` – `7.3+` | Legacy KMS | X11 / Legacy KMS |

---

## 4. Real Hardware Verification

Verified on physical Kepler hardware running **Linux Kernel 7.0 (`7.0.0-38-generic`)**:

- **Test Machine:** Lenovo Yoga 500-14IHW (Hybrid MUXless Optimus)
- **Discrete GPU:** NVIDIA Corporation `GK208BM` [GeForce 920M] (`10de:1299`, rev a1)
- **Integrated GPU:** Intel Corporation Haswell-ULT Integrated Graphics (`8086:0a16`, `i915`)
- **Driver Branch:** `470.256.02`
- **Kernel Version:** `7.0.0-38-generic` (x86_64)
- **Verified State:** `Kernel driver in use: nvidia` (`nvidia_drm`, `nvidia`) with `nvidia_drm.modeset = Y`

<img width="798" height="299" alt="Chụp màn hình từ 2026-10-30 09-31-40" src="https://github.com/user-attachments/assets/b365376e-df63-4085-a6b2-52fec1a9c547" />

---

## 5. Repository Layout

```text
nvidia-legacy-compat/
├── kernel-compat/
│   └── include/
│       └── nv-linux-6x-compat.h                  # Unified Kernel 6.0 -> 7.3+ compatibility header
├── patches/
│   ├── patch-matrix.json                         # Declarative SemVer patch matrix
│   └── 470.256.02/
│       ├── 0001-kernel-6.3-vm-flags.patch
│       ├── 0002-kernel-6.5-gup-signature.patch
│       ├── 0003-kernel-6.10-follow-pfn-drm.patch
│       └── 0004-kernel-6.15-to-7.x-compat.patch
├── gbm-shim/
│   ├── Cargo.toml                                # Rust cdylib crate manifest
│   ├── build.rs                                  # Scoped cdylib SONAME linker configuration
│   ├── linker.lds                                # Symbol versioning reference script
│   └── src/
│       ├── lib.rs                                # Mesa GBM C-ABI entrypoints & unit tests
│       ├── drm_ioctl.rs                          # DRM Dumb / GEM / PRIME UAPI wrappers
│       ├── sync.rs                               # DMA-BUF sync_file & fence polling engine
│       └── egl_shim.rs                           # EGL_PLATFORM_GBM_KHR -> DEVICE_EXT interposer
├── scripts/
│   └── auto-patch.sh                             # Automated .run extractor, patcher & DKMS builder
├── tests/
│   └── test_auto_patch.sh                        # Multi-kernel (6.4, 6.12, 7.3) patch test suite
└── .github/workflows/
    ├── ci.yml                                    # Continuous Integration (GCC, Rust, C-ABI, ShellCheck)
    └── release.yml                               # Automated GitHub Release binary bundler
```

---

## 6. Prerequisites

- **System Packages:** `bash`, `jq`, `patch`, `sed`, `dkms`, `make`, `gcc`, `linux-headers-$(uname -r)`
- **Rust Toolchain:** `cargo` and `rustc` (1.77+ stable)

**Debian / Ubuntu:**
```bash
sudo apt-get update
sudo apt-get install -y build-essential dkms linux-headers-$(uname -r) jq patch cargo rustc
```

**Arch Linux:**
```bash
sudo pacman -S --needed base-devel dkms linux-headers jq patch rust
```

---

## 7. Installation & Usage

### Step 1: Build and Install the Wayland GBM & EGL Shim (`libgbm.so.1`)

```bash
cd gbm-shim
cargo build --release

sudo install -Dm755 target/release/libgbm.so /usr/local/lib/nvidia-legacy-gbm/libgbm.so.1
sudo ln -sf libgbm.so.1 /usr/local/lib/nvidia-legacy-gbm/libgbm.so
```

### Step 2: Patch and Install the NVIDIA `.run` Driver via DKMS

Download the official NVIDIA `.run` installer (e.g., `NVIDIA-Linux-x86_64-470.256.02.run`), then execute `scripts/auto-patch.sh`:

```bash
chmod +x scripts/auto-patch.sh

# Optional: Verify patch applicability in dry-run mode
./scripts/auto-patch.sh \
  --driver-version 470.256.02 \
  --run-file ./NVIDIA-Linux-x86_64-470.256.02.run \
  --dry-run

# Apply patches, enable nvidia-drm.modeset=1, and build/install via DKMS
sudo ./scripts/auto-patch.sh \
  --driver-version 470.256.02 \
  --run-file ./NVIDIA-Linux-x86_64-470.256.02.run
```

---

## 8. Wayland Compositor Configuration

To run your Wayland compositor using the `nvidia-legacy-gbm` shim without overwriting your distribution's default Mesa packages, export the following environment variables before starting the compositor:

```bash
export LD_LIBRARY_PATH="/usr/local/lib/nvidia-legacy-gbm:${LD_LIBRARY_PATH:-}"
export LD_PRELOAD="/usr/local/lib/nvidia-legacy-gbm/libgbm.so.1"
export GBM_BACKEND="nvidia-legacy-drm-gbm"
export __GLX_VENDOR_LIBRARY_NAME="nvidia"
export WLR_NO_HARDWARE_CURSORS=1
export KWIN_DRM_USE_MODIFIERS=0
```

Verify that kernel modesetting is active after rebooting:
```bash
sudo cat /sys/module/nvidia_drm/parameters/modeset
# Expected output: Y
```

---

## 9. Development & Automated Verification

Run the automated test suites locally:

```bash
# 1. Lint Bash scripts and run multi-kernel (6.4, 6.12, 7.3) patch tests
shellcheck scripts/auto-patch.sh tests/test_auto_patch.sh
chmod +x scripts/auto-patch.sh tests/test_auto_patch.sh
./tests/test_auto_patch.sh

# 2. Run Rust GBM C-ABI, Fence Sync, and EGL Interposer unit tests
cd gbm-shim
cargo clippy --all-targets -- -D warnings
cargo test -- --nocapture
```

Our GitHub Actions CI pipeline (`.github/workflows/ci.yml`) automatically verifies:
1. **ShellCheck & JSON Matrix Integrity** across all scripts and patch definitions.
2. **Multi-Kernel Patch Application** against simulated Kernel `6.4.12`, `6.12.1`, and `7.3.1` trees.
3. **GCC `-Wall -Wextra -Werror` Compilation** of `nv-linux-6x-compat.h` across 8 distinct `LINUX_VERSION_CODE` targets (`6.1.0` through `7.3.0`).
4. **Rust Unit Tests & C Wayland Compositor Simulation** dynamically linked against `libgbm.so.1`.

---

## 10. License

Released under the **MIT License**. Proprietary NVIDIA installer archives downloaded by the user remain governed by the official NVIDIA Software License Agreement.
