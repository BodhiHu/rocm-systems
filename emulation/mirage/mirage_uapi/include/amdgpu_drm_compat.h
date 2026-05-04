#ifndef MIRAGE_UAPI_AMDGPU_DRM_COMPAT_H
#define MIRAGE_UAPI_AMDGPU_DRM_COMPAT_H

#include "libdrm/amdgpu_drm.h"

#ifndef DRM_AMDGPU_GEM_LIST_HANDLES
#define DRM_AMDGPU_GEM_LIST_HANDLES 0x19
#define DRM_IOCTL_AMDGPU_GEM_LIST_HANDLES \
  DRM_IOWR(DRM_COMMAND_BASE + DRM_AMDGPU_GEM_LIST_HANDLES, struct drm_amdgpu_gem_list_handles)

#define AMDGPU_GEM_LIST_HANDLES_FLAG_IS_IMPORT (1 << 0)

struct drm_amdgpu_gem_list_handles {
  __u64 entries;
  __u32 num_entries;
  __u32 padding;
};

struct drm_amdgpu_gem_list_handles_entry {
  __u32 gem_handle;
  __u32 flags;
  __u64 size;
  __u64 preferred_domains;
  __u64 alloc_flags;
  __u64 alignment;
};
#endif

#ifndef DRM_AMDGPU_SEM
#define DRM_AMDGPU_SEM 0x5b
#define DRM_IOCTL_AMDGPU_SEM DRM_IOWR(DRM_COMMAND_BASE + DRM_AMDGPU_SEM, union drm_amdgpu_sem)

#define AMDGPU_SEM_OP_CREATE_SEM 1
#define AMDGPU_SEM_OP_WAIT_SEM 2
#define AMDGPU_SEM_OP_SIGNAL_SEM 3
#define AMDGPU_SEM_OP_DESTROY_SEM 4
#define AMDGPU_SEM_OP_IMPORT_SEM 5
#define AMDGPU_SEM_OP_EXPORT_SEM 6

struct drm_amdgpu_sem_in {
  __u32 op;
  __u32 handle;
  __u32 ctx_id;
  __u32 ip_type;
  __u32 ip_instance;
  __u32 ring;
  __u64 seq;
};

union drm_amdgpu_sem_out {
  int fd;
  __u32 handle;
};

union drm_amdgpu_sem {
  struct drm_amdgpu_sem_in in;
  union drm_amdgpu_sem_out out;
};
#endif

#ifndef DRM_AMDGPU_GEM_DGMA
#define DRM_AMDGPU_GEM_DGMA 0x5c
#define DRM_IOCTL_AMDGPU_GEM_DGMA \
  DRM_IOWR(DRM_COMMAND_BASE + DRM_AMDGPU_GEM_DGMA, struct drm_amdgpu_gem_dgma)

#define AMDGPU_GEM_DGMA_IMPORT 0
#define AMDGPU_GEM_DGMA_QUERY_PHYS_ADDR 1

struct drm_amdgpu_gem_dgma {
  __u64 addr;
  __u64 size;
  __u32 op;
  __u32 handle;
};
#endif

#endif