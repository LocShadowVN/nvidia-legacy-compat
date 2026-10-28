#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# auto-patch.sh - Automated Kernel 6.x/7.x Patcher & DKMS Builder for Legacy NVIDIA Drivers

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
MATRIX_FILE="${PROJECT_ROOT}/patches/patch-matrix.json"
PATCHES_DIR="${PROJECT_ROOT}/patches"
COMPAT_HEADER="${PROJECT_ROOT}/kernel-compat/include/nv-linux-6x-compat.h"

DRIVER_VERSION=""
KERNEL_VERSION="$(uname -r)"
RUN_FILE=""
SOURCE_DIR=""
WORK_DIR="/tmp/nvidia-legacy-build-$$"
DRY_RUN=0
SKIP_DKMS=0

log_info()  { printf "\033[1;34m[INFO]\033[0m %s\n" "$*"; }
log_ok()    { printf "\033[1;32m[OK]\033[0m   %s\n" "$*"; }
log_warn()  { printf "\033[1;33m[WARN]\033[0m %s\n" "$*" >&2; }
log_error() { printf "\033[1;31m[ERR]\033[0m  %s\n" "$*" >&2; }

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Options:
  --driver-version <VER>   Target NVIDIA driver version (e.g., 470.256.02, 390.157, 340.108)
  --kernel-version <VER>   Target Kernel version (default: current uname -r)
  --run-file <PATH>        Path to NVIDIA-Linux-x86_64-<VER>.run installer
  --source-dir <PATH>      Use an already extracted NVIDIA directory instead of --run-file
  --matrix <PATH>          Custom path to patch-matrix.json
  --work-dir <PATH>        Working directory for extraction
  --dry-run                Validate patches without modifying files or running DKMS
  --skip-dkms              Apply patches and inject compat headers without registering DKMS
  -h, --help               Show this help message
EOF
}

# Chuẩn hóa chuỗi kernel an toàn tuyệt đối (VD: "7.2.9-arch1-1" -> "7.2.9", "7.0-rc1" -> "7.0.0")
normalize_semver() {
    local raw="$1"
    local clean="${raw%%-*}"
    local major="" minor="" patch=""
    IFS='.' read -r major minor patch _ <<< "${clean}"
    major="${major//[^0-9]/}"
    minor="${minor//[^0-9]/}"
    patch="${patch//[^0-9]/}"
    major="${major:-0}"
    minor="${minor:-0}"
    patch="${patch:-0}"
    printf "%d.%d.%d" "${major}" "${minor}" "${patch}"
}

semver_ge() {
    local v1 v2
    v1="$(normalize_semver "$1")"
    v2="$(normalize_semver "$2")"
    local m1 n1 p1 m2 n2 p2
    IFS='.' read -r m1 n1 p1 <<< "${v1}"
    IFS='.' read -r m2 n2 p2 <<< "${v2}"

    if (( m1 != m2 )); then (( m1 > m2 )); return $?; fi
    if (( n1 != n2 )); then (( n1 > n2 )); return $?; fi
    (( p1 >= p2 ))
}

semver_le() {
    local v1 v2
    v1="$(normalize_semver "$1")"
    v2="$(normalize_semver "$2")"
    local m1 n1 p1 m2 n2 p2
    IFS='.' read -r m1 n1 p1 <<< "${v1}"
    IFS='.' read -r m2 n2 p2 <<< "${v2}"

    if (( m1 != m2 )); then (( m1 < m2 )); return $?; fi
    if (( n1 != n2 )); then (( n1 < n2 )); return $?; fi
    (( p1 <= p2 ))
}

check_dependencies() {
    local deps=("jq" "patch" "sed")
    if (( SKIP_DKMS == 0 && DRY_RUN == 0 )); then
        deps+=("dkms")
    fi
    for cmd in "${deps[@]}"; do
        if ! command -v "${cmd}" >/dev/null 2>&1; then
            log_error "Missing required command: ${cmd}"
            exit 1
        fi
    done
}

parse_args() {
    while (( $# > 0 )); do
        case "$1" in
            --driver-version) DRIVER_VERSION="${2:-}"; shift 2 ;;
            --kernel-version) KERNEL_VERSION="${2:-}"; shift 2 ;;
            --run-file)       RUN_FILE="${2:-}"; shift 2 ;;
            --source-dir)     SOURCE_DIR="${2:-}"; shift 2 ;;
            --matrix)         MATRIX_FILE="${2:-}"; shift 2 ;;
            --work-dir)       WORK_DIR="${2:-}"; shift 2 ;;
            --dry-run)        DRY_RUN=1; shift ;;
            --skip-dkms)      SKIP_DKMS=1; shift ;;
            -h|--help)        usage; exit 0 ;;
            *)                log_error "Unknown argument: $1"; usage; exit 1 ;;
        esac
    done

    if [[ -z "${DRIVER_VERSION}" ]]; then
        log_error "--driver-version is required."
        usage
        exit 1
    fi
    if [[ ! -f "${MATRIX_FILE}" ]]; then
        log_error "Patch matrix file not found: ${MATRIX_FILE}"
        exit 1
    fi
}

resolve_patches() {
    local driver="$1"
    local kver
    kver="$(normalize_semver "$2")"

    if ! jq -e --arg drv "${driver}" '.drivers[$drv] != null' "${MATRIX_FILE}" >/dev/null; then
        log_error "Driver version ${driver} is not defined in ${MATRIX_FILE}"
        exit 1
    fi

    local min_supported max_supported
    min_supported="$(jq -r --arg drv "${driver}" '.drivers[$drv].supported_kernels.min' "${MATRIX_FILE}")"
    max_supported="$(jq -r --arg drv "${driver}" '.drivers[$drv].supported_kernels.max' "${MATRIX_FILE}")"

    if ! semver_ge "${kver}" "${min_supported}" || ! semver_le "${kver}" "${max_supported}"; then
        log_error "Kernel ${kver} is outside supported range [${min_supported}, ${max_supported}] for driver ${driver}"
        exit 1
    fi

    local count
    count="$(jq -r --arg drv "${driver}" '.drivers[$drv].patches | length' "${MATRIX_FILE}")"

    for (( i=0; i<count; i++ )); do
        local p_id p_file p_min p_max
        p_id="$(jq -r --arg drv "${driver}" --argjson idx "${i}" '.drivers[$drv].patches[$idx].id' "${MATRIX_FILE}")"
        p_file="$(jq -r --arg drv "${driver}" --argjson idx "${i}" '.drivers[$drv].patches[$idx].file' "${MATRIX_FILE}")"
        p_min="$(jq -r --arg drv "${driver}" --argjson idx "${i}" '.drivers[$drv].patches[$idx].min_kernel' "${MATRIX_FILE}")"
        p_max="$(jq -r --arg drv "${driver}" --argjson idx "${i}" '.drivers[$drv].patches[$idx].max_kernel' "${MATRIX_FILE}")"

        if semver_ge "${kver}" "${p_min}" && semver_le "${kver}" "${p_max}"; then
            printf "%s|%s\n" "${p_id}" "${PATCHES_DIR}/${p_file}"
        fi
    done
}

extract_installer() {
    if [[ -n "${SOURCE_DIR}" ]]; then
        if [[ ! -d "${SOURCE_DIR}" ]]; then
            log_error "Specified --source-dir does not exist: ${SOURCE_DIR}"
            exit 1
        fi
        printf "%s" "${SOURCE_DIR}"
        return 0
    fi

    if [[ -z "${RUN_FILE}" ]]; then
        log_error "Either --run-file or --source-dir must be provided."
        exit 1
    fi
    if [[ ! -f "${RUN_FILE}" ]]; then
        log_error "NVIDIA installer .run file not found: ${RUN_FILE}"
        exit 1
    fi

    mkdir -p "${WORK_DIR}"
    local target_extract="${WORK_DIR}/extracted"
    rm -rf "${target_extract}"

    log_info "Extracting ${RUN_FILE} to ${target_extract}..." >&2
    sh "${RUN_FILE}" --extract-only --target "${target_extract}" >/dev/null
    printf "%s" "${target_extract}"
}

# Tự động nâng cấp Kbuild & conftest.sh trên bộ nguồn .run thật cho Kernel 6.15+ và 7.x
apply_real_kbuild_and_conftest_fixes() {
    local target_dir="$1"
    (( DRY_RUN == 1 )) && return 0

    local kbuild_file="${target_dir}/kernel/Kbuild"
    if [[ -f "${kbuild_file}" ]]; then
        # Kernel 6.15+ và 7.x đã xóa EXTRA_CFLAGS, bắt buộc chuyển sang ccflags-y
        sed -i 's/\bEXTRA_CFLAGS\b/ccflags-y/g' "${kbuild_file}"
        if ! grep -q "nv-linux-6x-compat.h" "${kbuild_file}"; then
            cat >> "${kbuild_file}" <<'EOF'

# Injected by nvidia-legacy-compat for Kernel 6.x / 7.x
ccflags-y += -include $(src)/common/inc/nv-linux-6x-compat.h
ccflags-y += -Wno-error=implicit-function-declaration -Wno-error=strict-prototypes -fms-extensions
EOF
            log_ok "Updated Kbuild (EXTRA_CFLAGS -> ccflags-y & force-included nv-linux-6x-compat.h)"
        fi
    fi

    local conftest_file="${target_dir}/kernel/conftest.sh"
    if [[ -f "${conftest_file}" ]]; then
        if ! grep -q "fms-extensions" "${conftest_file}"; then
            sed -i 's/CFLAGS="/CFLAGS="-fms-extensions -Wno-implicit-function-declaration -Wno-strict-prototypes /g' "${conftest_file}"
            log_ok "Patched conftest.sh with GCC 14/15/16 & Kernel 6.19+/7.x flags (-fms-extensions)"
        fi
    fi
}

apply_patches_to_tree() {
    local target_dir="$1"
    shift
    local patches=("$@")

    if [[ -f "${COMPAT_HEADER}" && -d "${target_dir}/kernel/common/inc" ]]; then
        if (( DRY_RUN == 0 )); then
            cp -f "${COMPAT_HEADER}" "${target_dir}/kernel/common/inc/nv-linux-6x-compat.h"
            log_ok "Injected nv-linux-6x-compat.h into kernel/common/inc/"
        else
            log_info "[DRY-RUN] Would copy nv-linux-6x-compat.h into ${target_dir}/kernel/common/inc/"
        fi
    fi

    apply_real_kbuild_and_conftest_fixes "${target_dir}"

    for item in "${patches[@]}"; do
        [[ -z "${item}" ]] && continue
        local patch_id="${item%%|*}"
        local patch_path="${item#*|}"

        if [[ ! -f "${patch_path}" ]]; then
            log_error "Patch file missing for ${patch_id}: ${patch_path}"
            exit 1
        fi

        log_info "Validating ${patch_id} (${patch_path})..."
        if ! patch -d "${target_dir}" -p1 --fuzz=3 --dry-run --forward --silent < "${patch_path}"; then
            log_error "Dry-run failed for patch ${patch_id} on ${target_dir}"
            exit 1
        fi

        if (( DRY_RUN == 0 )); then
            patch -d "${target_dir}" -p1 --fuzz=3 --forward < "${patch_path}"
            log_ok "Applied patch ${patch_id}"
        else
            log_ok "[DRY-RUN] Verified patch ${patch_id} cleanly"
        fi
    done
}

configure_wayland_modeset() {
    if (( DRY_RUN == 1 || SKIP_DKMS == 1 )); then
        log_info "Skipping /etc/modprobe.d configuration (dry-run or skip-dkms active)"
        return 0
    fi

    if (( EUID != 0 )); then
        log_warn "Not running as root; skipping /etc/modprobe.d/nvidia-legacy-wayland.conf write"
        return 0
    fi

    local modprobe_conf="/etc/modprobe.d/nvidia-legacy-wayland.conf"
    cat > "${modprobe_conf}" <<'EOF'
# Generated by nvidia-legacy-compat
options nvidia-drm modeset=1 fbdev=1
options nvidia NVreg_PreserveVideoMemoryAllocations=1
blacklist nouveau
EOF
    log_ok "Configured nvidia-drm.modeset=1 and blacklisted nouveau in ${modprobe_conf}"
}

register_dkms() {
    local extracted_dir="$1"
    if (( DRY_RUN == 1 || SKIP_DKMS == 1 )); then
        log_info "Skipping DKMS registration."
        return 0
    fi

    if (( EUID != 0 )); then
        log_error "Root privileges are required for DKMS registration."
        exit 1
    fi

    local dkms_src="/usr/src/nvidia-${DRIVER_VERSION}"
    log_info "Installing patched kernel module source to ${dkms_src}..."
    rm -rf "${dkms_src}"
    mkdir -p "${dkms_src}"
    cp -a "${extracted_dir}/kernel/." "${dkms_src}/"

    if [[ ! -f "${dkms_src}/dkms.conf" ]]; then
        cat > "${dkms_src}/dkms.conf" <<EOF
PACKAGE_NAME="nvidia"
PACKAGE_VERSION="${DRIVER_VERSION}"
CLEAN="make clean"
BUILT_MODULE_NAME[0]="nvidia"
DEST_MODULE_LOCATION[0]="/kernel/drivers/video"
MAKE[0]="make -j\$(nproc) NV_EXCLUDE_BUILD_MODULES='' KERNEL_UNAME=\${kernelver} IGNORE_CC_MISMATCH=1 modules"
BUILT_MODULE_NAME[1]="nvidia-modeset"
DEST_MODULE_LOCATION[1]="/kernel/drivers/video"
BUILT_MODULE_NAME[2]="nvidia-drm"
DEST_MODULE_LOCATION[2]="/kernel/drivers/video"
BUILT_MODULE_NAME[3]="nvidia-uvm"
DEST_MODULE_LOCATION[3]="/kernel/drivers/video"
AUTOINSTALL="yes"
EOF
    fi

    dkms add -m nvidia -v "${DRIVER_VERSION}" || true
    dkms build -m nvidia -v "${DRIVER_VERSION}" -k "${KERNEL_VERSION}"
    dkms install -m nvidia -v "${DRIVER_VERSION}" -k "${KERNEL_VERSION}"
    log_ok "DKMS build and installation succeeded for nvidia/${DRIVER_VERSION} on kernel ${KERNEL_VERSION}"
}

main() {
    parse_args "$@"
    check_dependencies

    local norm_kver
    norm_kver="$(normalize_semver "${KERNEL_VERSION}")"
    log_info "Target Driver: ${DRIVER_VERSION} | Target Kernel: ${KERNEL_VERSION} (SemVer: ${norm_kver})"

    mapfile -t applicable_patches < <(resolve_patches "${DRIVER_VERSION}" "${KERNEL_VERSION}")
    log_info "Resolved ${#applicable_patches[@]} applicable patch(es) from matrix."

    local target_tree
    target_tree="$(extract_installer)"
    apply_patches_to_tree "${target_tree}" "${applicable_patches[@]}"
    configure_wayland_modeset
    register_dkms "${target_tree}"

    log_ok "nvidia-legacy-compat pipeline completed successfully."
}

main "$@"
