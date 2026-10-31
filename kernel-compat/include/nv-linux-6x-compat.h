/* SPDX-License-Identifier: MIT */
/*
 * nv-linux-6x-compat.h
 * Production compatibility shim for Legacy NVIDIA Drivers (470.xx, 390.xx, 340.xx)
 * Supports Linux Kernel 6.0 through 7.4+
 */

#ifndef _NV_LINUX_6X_COMPAT_H_
#define _NV_LINUX_6X_COMPAT_H_

#include <linux/version.h>
#include <linux/mm.h>
#include <linux/sched/mm.h>
#include <linux/timer.h>
#include <linux/string.h>
#include <linux/interrupt.h>
#include <linux/dma-fence.h>

#if LINUX_VERSION_CODE >= KERNEL_VERSION(7, 0, 0)
#include <linux/sysfb.h>
#endif

#if LINUX_VERSION_CODE >= KERNEL_VERSION(7, 4, 0)
#include <linux/uaccess.h>
#endif

/*
 * 1. Quản lý cờ VMA (Có guard chống định nghĩa trùng với nv-linux.h):
 * - Linux 6.3 -> 6.14: dùng vm_flags_set() / vm_flags_clear()
 * - Linux 6.15+ & 7.x: dùng vm_flags_reset() để tránh symbol GPL-only __vma_start_write
 */
#ifndef NV_VM_FLAGS_HELPERS_DEFINED
#define NV_VM_FLAGS_HELPERS_DEFINED 1
static inline void nv_vm_flags_set(struct vm_area_struct *vma, vm_flags_t flags)
{
#if LINUX_VERSION_CODE >= KERNEL_VERSION(6, 15, 0)
    vm_flags_reset(vma, vma->vm_flags | flags);
#elif LINUX_VERSION_CODE >= KERNEL_VERSION(6, 3, 0)
    vm_flags_set(vma, flags);
#else
    vma->vm_flags |= flags;
#endif
}

static inline void nv_vm_flags_clear(struct vm_area_struct *vma, vm_flags_t flags)
{
#if LINUX_VERSION_CODE >= KERNEL_VERSION(6, 15, 0)
    vm_flags_reset(vma, vma->vm_flags & ~flags);
#elif LINUX_VERSION_CODE >= KERNEL_VERSION(6, 3, 0)
    vm_flags_clear(vma, flags);
#else
    vma->vm_flags &= ~flags;
#endif
}
#endif /* NV_VM_FLAGS_HELPERS_DEFINED */

/*
 * 2. Linux 6.5+ & 7.x: Loại bỏ tham số `struct vm_area_struct **vmas`
 * khỏi get_user_pages(), pin_user_pages(), và get_user_pages_remote()
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
 * 3. Linux 6.10+ / 6.12+ / 7.x: Thay thế follow_pfn() bằng follow_pfnmap_start()
 */
#ifndef NV_FOLLOW_PFN_HELPER_DEFINED
#define NV_FOLLOW_PFN_HELPER_DEFINED 1
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
#endif /* NV_FOLLOW_PFN_HELPER_DEFINED */

/*
 * 4. Linux 6.8+: Cờ DRM_UNLOCKED bị gỡ bỏ
 */
#ifndef DRM_UNLOCKED
#define DRM_UNLOCKED 0
#endif

/*
 * 5. Linux 6.15+ & 7.x: del_timer() / del_timer_sync() chuyển sang timer_delete()
 */
#if LINUX_VERSION_CODE >= KERNEL_VERSION(6, 15, 0)
#ifndef del_timer
#define del_timer(t) timer_delete(t)
#endif
#ifndef del_timer_sync
#define del_timer_sync(t) timer_delete_sync(t)
#endif
#endif

/*
 * 6. Linux 6.19+ & 7.x: in_irq() chuyển sang in_hardirq()
 */
#if LINUX_VERSION_CODE >= KERNEL_VERSION(6, 19, 0)
#ifndef in_irq
#define in_irq() in_hardirq()
#endif
#endif

/*
 * 7. Linux 7.0+:
 * - screen_info chuyển thành sysfb_primary_display.screen
 * - dma_fence_signal() trả về void
 */
static inline const struct screen_info *nv_get_screen_info(void)
{
#if LINUX_VERSION_CODE >= KERNEL_VERSION(7, 0, 0)
    return &sysfb_primary_display.screen;
#else
    return &screen_info;
#endif
}

static inline int nv_dma_fence_signal(struct dma_fence *fence)
{
#if LINUX_VERSION_CODE >= KERNEL_VERSION(7, 0, 0)
    dma_fence_signal(fence);
    return 0;
#else
    return dma_fence_signal(fence);
#endif
}

/*
 * 8. Linux 7.2+ / 7.3+: strncpy() bị xóa khỏi kernel, thay bằng strscpy()
 */
#if LINUX_VERSION_CODE >= KERNEL_VERSION(7, 2, 0)
static inline char *nv_compat_strncpy(char *dest, const char *src, size_t count)
{
    if (count > 0)
        strscpy(dest, src, count);
    return dest;
}
#ifndef strncpy
#define strncpy(d, s, n) nv_compat_strncpy((d), (s), (n))
#endif
#endif

/*
 * 9. Linux 7.4+: Các macro không an toàn __get_user() và __put_user()
 * bị gỡ bỏ khỏi <linux/uaccess.h>, ánh xạ sang get_user() và put_user().
 */
#if LINUX_VERSION_CODE >= KERNEL_VERSION(7, 4, 0)
#ifndef __get_user
#define __get_user(x, ptr) get_user((x), (ptr))
#endif
#ifndef __put_user
#define __put_user(x, ptr) put_user((x), (ptr))
#endif
#endif

#endif /* _NV_LINUX_6X_COMPAT_H_ */
