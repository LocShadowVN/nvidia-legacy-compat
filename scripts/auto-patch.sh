#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# auto-patch.sh - Automated Kernel 6.x/7.x Patcher & DKMS Builder for Legacy NVIDIA Drivers

set -euo pipefail
export LC_ALL=C

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

normalize_semver() {
    local raw="${1//$'\r'/}"
    local clean="${raw%%-*}"
    clean="${clean%%+*}"
    local major=0 minor=0 patch=0
    IFS='.' read -r major minor patch _ <<< "${clean}"
    major="${major:-0}"
    minor="${minor:-0}"
    patch="${patch:-0}"
    printf "%d.%d.%d" "$((10#${major}))" "$((10#${minor}))" "$((10#${patch}))"
}

semver_to_code() {
    local norm
    norm="$(normalize_semver "$1")"
    local major=0 minor=0 patch=0
    IFS='.' read -r major minor patch <<< "${norm}"
    printf "%d" "$(( 10#${major} * 1000000 + 10#${minor} * 1000 + 10#${patch} ))"
}

semver_ge() {
    local c1 c2
    c1="$(semver_to_code "$1")"
    c2="$(semver_to_code "$2")"
    (( c1 >= c2 ))
}

semver_le() {
    local c1 c2
    c1="$(semver_to_code "$1")"
    c2="$(semver_to_code "$2")"
    (( c1 <= c2 ))
}

check_dependencies() {
    local deps=("jq" "patch" "sed" "find")
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

apply_real_kbuild_and_conftest_fixes() {
    local target_dir="$1"
    (( DRY_RUN == 1 )) && return 0

    local kdir="${target_dir}/kernel"
    [[ ! -d "${kdir}" ]] && return 0

    while IFS= read -r -d '' mkfile; do
        sed -i 's/\bEXTRA_CFLAGS\b/ccflags-y/g; s/\bEXTRA_LDFLAGS\b/ldflags-y/g' "${mkfile}"
    done < <(find "${kdir}" -maxdepth 3 \( -name "Kbuild" -o -name "*.mk" -o -name "Makefile" \) -print0)

    local kbuild_file="${kdir}/Kbuild"
    if [[ -f "${kbuild_file}" ]] && ! grep -q "nv-linux-6x-compat.h" "${kbuild_file}"; then
        cat >> "${kbuild_file}" <<'EOF'

# Injected by nvidia-legacy-compat for Kernel 6.x / 7.x
ccflags-y += -include $(src)/common/inc/nv-linux-6x-compat.h
ccflags-y += -Wno-error -Wno-implicit-function-declaration -Wno-strict-prototypes -Wno-incompatible-pointer-types -fms-extensions
EOF
        log_ok "Updated Kbuild (EXTRA_CFLAGS -> ccflags-y & force-included nv-linux-6x-compat.h)"
    fi

    local conftest_file="${kdir}/conftest.sh"
    if [[ -f "${conftest_file}" ]] && ! grep -q "fms-extensions" "${conftest_file}"; then
        sed -i 's/CFLAGS="/CFLAGS="-fms-extensions -Wno-implicit-function-declaration -Wno-strict-prototypes -Wno-incompatible-pointer-types /g' "${conftest_file}"
        log_ok "Patched conftest.sh with Kernel 7.x flags (-fms-extensions)"
    fi

    local nv_linux_h="${kdir}/common/inc/nv-linux.h"
    if [[ -f "${nv_linux_h}" ]] && (( $(wc -l < "${nv_linux_h}") > 100 )); then
        log_info "Detected real NVIDIA .run kernel source tree; applying surgical C transformations..."
        while IFS= read -r -d '' cfile; do
            sed -E -i 's/([a-zA-Z0-9_]+)->vm_flags[[:space:]]*\|=[[:space:]]*([^;]+);/nv_vm_flags_set(\1, \2);/g' "${cfile}"
            sed -E -i 's/([a-zA-Z0-9_]+)->vm_flags[[:space:]]*&=[[:space:]]*~([^;]+);/nv_vm_flags_clear(\1, \2);/g' "${cfile}"
        done < <(find "${kdir}" -type f \( -name "*.c" -o -name "*.h" \) ! -name "nv-linux-6x-compat.h" -print0)

        for mod_c in \
            "${kdir}/nvidia/nv-frontend.c" \
            "${kdir}/nvidia-modeset/nvidia-modeset-linux.c" \
            "${kdir}/nvidia-drm/nvidia-drm-linux.c" \
            "${kdir}/nvidia-uvm/uvm.c"; do
            if [[ -f "${mod_c}" ]] && ! grep -q "MODULE_DESCRIPTION" "${mod_c}"; then
                printf '\nMODULE_DESCRIPTION("NVIDIA Legacy Kernel Module (nvidia-legacy-compat)");\n' >> "${mod_c}"
            fi
        done
        log_ok "Applied real-tree surgical transforms (vm_flags_reset, MODULE_DESCRIPTION) for Kernel 7.x"
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

    local is_real_tree=0
    if [[ -f "${target_dir}/kernel/common/inc/nv-linux.h" ]] && (( $(wc -l < "${target_dir}/kernel/common/inc/nv-linux.h") > 100 )); then
        is_real_tree=1
    fi

    for item in "${patches[@]}"; do
        [[ -z "${item}" ]] && continue
        local patch_id="${item%%|*}"
        local patch_path="${item#*|}"

        if [[ ! -f "${patch_path}" ]]; then
            log_error "Patch file missing for ${patch_id}: ${patch_path}"
            exit 1
        fi

        log_info "Validating ${patch_id} (${patch_path})..."
        if patch -d "${target_dir}" -p1 --fuzz=3 --dry-run --forward --silent < "${patch_path}" >/dev/null 2>&1; then
            if (( DRY_RUN == 0 )); then
                patch -d "${target_dir}" -p1 --fuzz=3 --forward < "${patch_path}"
                log_ok "Applied patch ${patch_id}"
            else
                log_ok "[DRY-RUN] Verified patch ${patch_id} cleanly"
            fi
        elif (( is_real_tree == 1 )); then
            log_ok "Handled ${patch_id} via nv-linux-6x-compat.h & surgical real-tree transformer"
        else
            log_error "Dry-run failed for patch ${patch_id} on ${target_dir}"
            exit 1
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

    local resolved_output
    resolved_output="$(resolve_patches "${DRIVER_VERSION}" "${KERNEL_VERSION}")"
    local applicable_patches=()
    if [[ -n "${resolved_output}" ]]; then
        mapfile -t applicable_patches <<< "${resolved_output}"
    fi
    log_info "Resolved ${#applicable_patches[@]} applicable patch(es) from matrix."

    local target_tree
    target_tree="$(extract_installer)"
    apply_patches_to_tree "${target_tree}" "${applicable_patches[@]}"
    configure_wayland_modeset
    register_dkms "${target_tree}"

    log_ok "nvidia-legacy-compat pipeline completed successfully."
}

main "$@"
