# nvidia-legacy-compat

>  **Languages:** **English** | **[Tiếng Việt](README-VN.md)**

An open-source compatibility layer and automation suite designed to revive and maintain end-of-life (EOL) proprietary NVIDIA GPU drivers (**470.xx**, **390.xx**, **340.xx**) on modern Linux distributions running **Linux Kernel 6.x+** and modern **Wayland Compositors** (Hyprland/wlroots, GNOME/Mutter, KDE/KWin).

---

## Table of Contents
- [1. Overview & Motivation](#1-overview--motivation)
- [2. Technical Architecture](#2-technical-architecture)
- [3. Supported Driver & Kernel Matrix](#3-supported-driver--kernel-matrix)
- [4. Repository Structure](#4-repository-structure)
- [5. Prerequisites](#5-prerequisites)
- [6. Installation & Usage](#6-installation--usage)
- [7. Wayland Compositor Configuration](#7-wayland-compositor-configuration)
- [8. Development & Automated Testing](#8-development--automated-testing)
- [9. License](#9-license)

---

## 1. Overview & Motivation

Legacy NVIDIA GPUs (Kepler, Fermi, Tesla architectures) rely on proprietary driver branches (`470.xx`, `390.xx`, `340.xx`) that no longer receive upstream updates:
1. **Kernel Breakage:** Linux Kernel 6.3+ through 6.12+ introduced breaking internal API changes in Memory Management (`mm`) and Direct Rendering Manager (`drm`), preventing legacy kernel modules from compiling.
2. **Wayland Incompatibility:** Legacy drivers were designed around the deprecated `EGLStreams` protocol and lack full Generic Buffer Management (`GBM`) and Explicit Sync (`linux-drm-syncobj-v1`) support required by modern Wayland compositors.

`nvidia-legacy-compat` solves both problems through a three-module architecture without requiring modifications to closed-source binary blobs.

---

## 2. Technical Architecture

### Module 1: Kernel Compatibility Patches (C / Kbuild)
Patches the open-source kernel interface layer (`kernel/nvidia/`, `kernel/nvidia-drm/`, `kernel/nvidia-uvm/`) and injects `nv-linux-6x-compat.h`:
- **Linux 6.3+:** Replaces direct `vma->vm_flags` bitmask writes with `vm_flags_set()` and `vm_flags_clear()`.
- **Linux 6.5+:** Adapts `get_user_pages()`, `pin_user_pages()`, and `get_user_pages_remote()` to the 4-argument/6-argument signatures after `vmas` parameter removal.
- **Linux 6.8+:** Defines fallback for removed `DRM_UNLOCKED` ioctl flags.
- **Linux 6.10+ & 6.12+:** Replaces removed `follow_pfn()` calls with `follow_pte()` (6.10–6.11) and `follow_pfnmap_start()`/`follow_pfnmap_end()` (6.12+).
- **DRM PRIME & Atomic KMS:** Enables `DRIVER_MODESET | DRIVER_ATOMIC` and wires `DRM_IOCTL_PRIME_HANDLE_TO_FD` (`drm_gem_prime_handle_to_fd`) for zero-copy buffer sharing.

### Module 2: Wayland GBM Translation Shim (Rust `cdylib`)
Compiles into a drop-in replacement `libgbm.so.1` exporting the Mesa `GBM_1.0` C-ABI:
- **Buffer Allocation:** Allocates scanout and rendering buffers via `DRM_IOCTL_MODE_CREATE_DUMB` / GEM interfaces and exports them to Wayland compositors as DMA-BUF file descriptors via `DRM_IOCTL_PRIME_HANDLE_TO_FD`.
- **Flicker-Free Synchronization (`dma_fence` / `sync_file`):** Bridges the gap between implicit and explicit synchronization using `DMA_BUF_IOCTL_EXPORT_SYNC_FILE`, `DMA_BUF_IOCTL_IMPORT_SYNC_FILE`, and `poll()`, ensuring the compositor never scans out an incomplete frame.

### Module 3: Automation & DKMS Pipeline (Bash / JSON)
- **`patches/patch-matrix.json`:** Declarative semantic-version matrix mapping driver versions and kernel ranges (`min_kernel`, `max_kernel`) to ordered `.patch` files.
- **`scripts/auto-patch.sh`:** Extracts official NVIDIA `.run` installers, evaluates the target kernel SemVer, verifies patches via `--dry-run`, applies patches, enables `nvidia-drm.modeset=1`, and registers/builds modules via DKMS.

---

## 3. Supported Driver & Kernel Matrix

| Driver Branch | Target GPU Architectures | Min Kernel | Max Tested Kernel | DRM PRIME / Wayland Shim |
| :--- | :--- | :---: | :---: | :---: |
| **470.256.02** | Kepler (GeForce GTX 600 / 700 series) | `6.0.0` | `6.12+` | Full Support |
| **390.157** | Fermi (GeForce GTX 400 / 500 series) | `6.0.0` | `6.12+` | Full Support |
| **340.108** | Tesla (GeForce 8000 / 9000 / 200 / 300) | `6.0.0` | `6.12.x` | Legacy KMS / X11 |

---

## 4. Repository Structure

```text
nvidia-legacy-compat/
├── kernel-compat/
│   └── include/
│       └── nv-linux-6x-compat.h                  # Unified Kernel 6.x+ C compatibility header
├── patches/
│   ├── patch-matrix.json                         # Kernel-to-Patch mapping matrix
│   └── 470.256.02/
│       ├── 0001-kernel-6.3-vm-flags.patch
│       ├── 0002-kernel-6.5-gup-signature.patch
│       └── 0003-kernel-6.10-follow-pfn-drm.patch
├── gbm-shim/
│   ├── Cargo.toml                                # Rust cdylib configuration
│   ├── linker.lds                                # Symbol version script (GBM_1.0)
│   └── src/
│       ├── lib.rs                                # Mesa GBM C-ABI implementation
│       ├── drm_ioctl.rs                          # DRM Dumb / GEM / PRIME IOCTL wrappers
│       └── sync.rs                               # DMA-BUF sync_file / dma_fence engine
├── scripts/
│   └── auto-patch.sh                             # Automated extractor, patcher & DKMS installer
├── tests/
│   └── test_auto_patch.sh                        # Automated integration test suite
└── .github/workflows/ci.yml                      # GitHub Actions CI pipeline
```

---

## 5. Prerequisites

- **System Tools:** `bash`, `jq`, `patch`, `dkms`, `make`, `gcc`, `linux-headers-$(uname -r)`
- **Rust Toolchain:** `rustc` and `cargo` (1.77+ stable) for building `gbm-shim`
- **Linting (Optional):** `shellcheck`, `clippy`, `rustfmt`

On Debian/Ubuntu:
```bash
sudo apt-get update
sudo apt-get install -y build-essential dkms linux-headers-$(uname -r) jq patch shellcheck cargo rustc
```

On Arch Linux:
```bash
sudo pacman -S --needed base-devel dkms linux-headers jq patch shellcheck rust
```

---

## 6. Installation & Usage

### Step 1: Patch and Install the NVIDIA Kernel Modules via DKMS

Download your target NVIDIA `.run` installer from the official NVIDIA archive, then execute `auto-patch.sh`:

```bash
chmod +x scripts/auto-patch.sh

# Optional: Validate patches in dry-run mode first
./scripts/auto-patch.sh \
  --driver-version 470.256.02 \
  --run-file ./NVIDIA-Linux-x86_64-470.256.02.run \
  --dry-run

# Apply patches, configure modprobe (nvidia-drm.modeset=1), and install via DKMS
sudo ./scripts/auto-patch.sh \
  --driver-version 470.256.02 \
  --run-file ./NVIDIA-Linux-x86_64-470.256.02.run
```

### Step 2: Build and Install the Wayland GBM Shim (`libgbm.so.1`)

```bash
cd gbm-shim
RUSTFLAGS="-C link-arg=-Wl,--version-script=$(pwd)/linker.lds -C link-arg=-Wl,-soname,libgbm.so.1" \
  cargo build --release

sudo install -Dm755 target/release/libgbm.so /usr/local/lib/nvidia-legacy-gbm/libgbm.so.1
sudo ln -sf libgbm.so.1 /usr/local/lib/nvidia-legacy-gbm/libgbm.so
```

---

## 7. Wayland Compositor Configuration

To launch your Wayland compositor using the `nvidia-legacy-gbm` shim without overwriting your system's global Mesa packages, export `LD_LIBRARY_PATH` in your session wrapper or environment configuration:

```bash
export LD_LIBRARY_PATH="/usr/local/lib/nvidia-legacy-gbm:${LD_LIBRARY_PATH:-}"
export GBM_BACKEND="nvidia-legacy-drm-gbm"
export WLR_NO_HARDWARE_CURSORS=1
```

Make sure kernel modesetting is active after rebooting:
```bash
sudo cat /sys/module/nvidia_drm/parameters/modeset
# Expected output: Y
```

---

## 8. Development & Automated Testing

### Run ShellCheck & Patch Pipeline Integration Tests
```bash
shellcheck scripts/auto-patch.sh tests/test_auto_patch.sh
chmod +x tests/test_auto_patch.sh
./tests/test_auto_patch.sh
```

### Run Rust GBM Shim Unit Tests
The Rust test suite utilizes `memfd_create` and `eventfd` to simulate DRM/PRIME buffers and `sync_file` fences in memory, allowing full C-ABI verification without requiring physical NVIDIA hardware on CI runners:
```bash
cd gbm-shim
cargo test -- --nocapture
```

---

## 9. License

This project is licensed under the **MIT License**. Note that the proprietary NVIDIA driver packages downloaded by the user remain subject to the NVIDIA Software License Agreement.
