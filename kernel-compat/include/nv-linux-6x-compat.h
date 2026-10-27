/* SPDX-License-Identifier: MIT */
/*
 * nv-linux-6x-compat.h
 * Compatibility shim for Legacy NVIDIA Drivers (470.xx, 390.xx, 340.xx)
 * on Linux Kernel 6.0 through 6.12+
 */

#ifndef _NV_LINUX_6X_COMPAT_H_
#define _NV_LINUX_6X_COMPAT_H_

#include <linux/version.h>
#include <linux/mm.h>
#include <linux/sched/mm.h>
#include <drm/drm_drv.h>
#include <drm/drm_ioctl.h>
#include <drm/drm_prime.h>
#include <drm/drm_fb_helper.h>

/*
 * 1. Linux 6.3+: vma->vm_flags chuyển thành biến hằng (__vm_flags)
 * Bắt buộc sử dụng vm_flags_set() và vm_flags_clear()
 */
static inline void nv_vm_flags_set(struct vm_area_struct *vma, vm_flags_t flags)
{
#if LINUX_VERSION_CODE >= KERNEL_VERSION(6, 3, 0)
    vm_flags_set(vma, flags);
#else
    vma->vm_flags |= flags;
#endif
}

static inline void nv_vm_flags_clear(struct vm_area_struct *vma, vm_flags_t flags)
{
#if LINUX_VERSION_CODE >= KERNEL_VERSION(6, 3, 0)
    vm_flags_clear(vma, flags);
#else
    vma->vm_flags &= ~flags;
#endif
}

/*
 * 2. Linux 6.5+: Loại bỏ tham số `struct vm_area_struct **vmas` khỏi
 * get_user_pages(), pin_user_pages(), và get_user_pages_remote()
 */
static inline long nv_get_user_pages(unsigned long start,
                                     unsigned long nr_pages,
                                     unsigned int gup_flags,
                                     struct page **pages,
                                     struct vm_area_struct **vmas)
{
#if LINUX_VERSION_CODE >= KERNEL_VERSION(6, 5, 0)
    (void)vmas;
    return get_user_pages(start, nr_pages, gup_flags, pages);
#else
    return get_user_pages(start, nr_pages, gup_flags, pages, vmas);
#endif
}

static inline long nv_pin_user_pages(unsigned long start,
                                     unsigned long nr_pages,
                                     unsigned int gup_flags,
                                     struct page **pages,
                                     struct vm_area_struct **vmas)
{
#if LINUX_VERSION_CODE >= KERNEL_VERSION(6, 5, 0)
    (void)vmas;
    return pin_user_pages(start, nr_pages, gup_flags, pages);
#else
    return pin_user_pages(start, nr_pages, gup_flags, pages, vmas);
#endif
}

static inline long nv_get_user_pages_remote(struct mm_struct *mm,
                                            unsigned long start,
                                            unsigned long nr_pages,
                                            unsigned int gup_flags,
                                            struct page **pages,
                                            struct vm_area_struct **vmas,
                                            int *locked)
{
#if LINUX_VERSION_CODE >= KERNEL_VERSION(6, 5, 0)
    (void)vmas;
    return get_user_pages_remote(mm, start, nr_pages, gup_flags, pages, locked);
#elif LINUX_VERSION_CODE >= KERNEL_VERSION(5, 9, 0)
    return get_user_pages_remote(mm, start, nr_pages, gup_flags, pages, vmas, locked);
#else
    return get_user_pages_remote(NULL, mm, start, nr_pages, gup_flags, pages, vmas, locked);
#endif
}

/*
 * 3. Linux 6.10+: Hàm follow_pfn() bị loại bỏ khỏi mm/memory.c
 * Thay thế bằng follow_pte() (6.10 - 6.11) hoặc follow_pfnmap_start() (6.12+)
 */
static inline int nv_follow_pfn(struct vm_area_struct *vma,
                                unsigned long address,
                                unsigned long *pfn)
{
#if LINUX_VERSION_CODE >= KERNEL_VERSION(6, 12, 0)
    struct follow_pfnmap_args args = {
        .vma = vma,
        .address = address,
    };
    int ret = follow_pfnmap_start(&args);
    if (ret)
        return ret;
    *pfn = args.pfn;
    follow_pfnmap_end(&args);
    return 0;
#elif LINUX_VERSION_CODE >= KERNEL_VERSION(6, 10, 0)
    pte_t *ptep = NULL;
    spinlock_t *ptl = NULL;
    int ret = follow_pte(vma, address, &ptep, &ptl);
    if (ret)
        return ret;
    *pfn = pte_pfn(ptep_get(ptep));
    pte_unmap_unlock(ptep, ptl);
    return 0;
#else
    return follow_pfn(vma, address, pfn);
#endif
}

/*
 * 4. Linux 6.8+: Cờ DRM_UNLOCKED bị gỡ bỏ vì mọi DRM IOCTL hiện đại
 * đều mặc định chạy không giữ drm_global_mutex.
 */
#ifndef DRM_UNLOCKED
#define DRM_UNLOCKED 0
#endif

#endif /* _NV_LINUX_6X_COMPAT_H_ */
