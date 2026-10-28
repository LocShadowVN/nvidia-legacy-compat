#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
AUTO_PATCH="${PROJECT_ROOT}/scripts/auto-patch.sh"

TEST_TMP="$(mktemp -d)"
trap 'rm -rf "${TEST_TMP}"' EXIT

create_mock_nvidia_tree() {
    local dst="$1"
    mkdir -p "${dst}/kernel/common/inc" \
             "${dst}/kernel/nvidia" \
             "${dst}/kernel/nvidia-drm" \
             "${dst}/kernel/nvidia-uvm"

    cat > "${dst}/kernel/common/inc/nv-linux.h" <<'EOF'
static inline void nv_vm_flags_set(struct vm_area_struct *vma, vm_flags_t flags)
{
    vma->vm_flags |= flags;
}

static inline void nv_vm_flags_clear(struct vm_area_struct *vma, vm_flags_t flags)
{
    vma->vm_flags &= ~flags;
}
EOF

    cat > "${dst}/kernel/nvidia/nv-mmap.c" <<'EOF'
        vma->vm_ops = &nv_vm_ops;
        vma->vm_private_data = nv;
        vma->vm_flags |= (VM_IO | VM_PFNMAP | VM_DONTEXPAND);
    }
EOF

    cat > "${dst}/kernel/nvidia-drm/nvidia-drm-gem-nvkms-memory.c" <<'EOF'
    vma->vm_flags &= ~VM_PFNMAP;
    vma->vm_flags &= ~VM_IO;
    vma->vm_flags |= VM_MIXEDMAP;
EOF

    cat > "${dst}/kernel/nvidia-uvm/uvm.c" <<'EOF'
    vma->vm_ops = &uvm_vm_ops_managed;
    vma->vm_flags |= VM_MIXEDMAP | VM_DONTEXPAND;
EOF

    cat > "${dst}/kernel/common/inc/nv-mm.h" <<'EOF'
                                struct page **pages,
                                struct vm_area_struct **vmas)
{
    return pin_user_pages(start, nr_pages, gup_flags, pages, vmas);
}

                                       struct vm_area_struct **vmas,
                                       int *locked)
{
    return get_user_pages_remote(mm, start, nr_pages, gup_flags, pages, vmas, locked);
}
EOF

    cat > "${dst}/kernel/nvidia/os-mlock.c" <<'EOF'
static inline int nv_follow_pfn(struct vm_area_struct *vma,
                                unsigned long address,
                                unsigned long *pfn)
{
    return follow_pfn(vma, address, pfn);
}
EOF

    cat > "${dst}/kernel/nvidia-drm/nvidia-drm-drv.c" <<'EOF'
static void nv_drm_output_poll_changed(struct drm_device *dev)
{
    /* Maintained for < 6.12 compatibility */
}

static const struct drm_mode_config_funcs nv_mode_config_funcs = {
    .fb_create = nv_drm_internal_framebuffer_create,
    .atomic_check = nv_drm_atomic_check,
    .atomic_commit = nv_drm_atomic_commit,
    .output_poll_changed = nv_drm_output_poll_changed,
};

static struct drm_driver nv_drm_driver = {
    .driver_features        = DRIVER_GEM | DRIVER_RENDER,
    .ioctls                 = nv_drm_ioctls,
    .num_ioctls             = ARRAY_SIZE(nv_drm_ioctls),
    .prime_handle_to_fd     = drm_gem_prime_handle_to_fd,
    .prime_fd_to_handle     = drm_gem_prime_fd_to_handle,
    .gem_prime_import       = nv_drm_gem_prime_import,
    .gem_prime_import_sg_table = nv_drm_gem_prime_import_sg_table,
EOF
}

echo "=== [TEST 1] Patching for Linux Kernel 6.4.12 (Only 6.3 VM flags patch should apply) ==="
TREE_64="${TEST_TMP}/tree_64"
create_mock_nvidia_tree "${TREE_64}"
bash "${AUTO_PATCH}" \
    --driver-version "470.256.02" \
    --kernel-version "6.4.12-arch1-1" \
    --source-dir "${TREE_64}" \
    --skip-dkms

grep -q "vm_flags_set(vma, flags);" "${TREE_64}/kernel/common/inc/nv-linux.h"
if grep -q "follow_pfnmap_start" "${TREE_64}/kernel/nvidia/os-mlock.c"; then
    echo "[FAIL] Patch 0003 (Kernel 6.8+) should not be applied to Kernel 6.4.12" >&2
    exit 1
fi
echo "[PASS] Kernel 6.4.12 selective patch test succeeded."

echo "=== [TEST 2] Patching for Linux Kernel 6.12.1 (All 3 patches must apply cleanly) ==="
TREE_612="${TEST_TMP}/tree_612"
create_mock_nvidia_tree "${TREE_612}"
bash "${AUTO_PATCH}" \
    --driver-version "470.256.02" \
    --kernel-version "6.12.1-generic" \
    --source-dir "${TREE_612}" \
    --skip-dkms

grep -q "vm_flags_set(vma, flags);" "${TREE_612}/kernel/common/inc/nv-linux.h"
grep -q "pin_user_pages(start, nr_pages, gup_flags, pages);" "${TREE_612}/kernel/common/inc/nv-mm.h"
grep -q "follow_pfnmap_start" "${TREE_612}/kernel/nvidia/os-mlock.c"
grep -q "DRIVER_MODESET | DRIVER_ATOMIC" "${TREE_612}/kernel/nvidia-drm/nvidia-drm-drv.c"
test -f "${TREE_612}/kernel/common/inc/nv-linux-6x-compat.h"
echo "[PASS] Kernel 6.12.1 full patch stack test succeeded."
