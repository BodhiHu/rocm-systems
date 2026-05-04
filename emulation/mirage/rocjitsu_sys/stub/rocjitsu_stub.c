// Copyright (c) 2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT
//
// Fallback stub definitions for every symbol exported by the rocjitsu C
// API. `rocjitsu_sys/build.rs` compiles and links this file when no
// real `librocjitsu` is available (i.e. `ROCJITSU_LIB_DIR` is unset),
// so downstream crates can at least *build and link* on hosts without
// rocjitsu. Every stub returns ROCJITSU_STATUS_ERROR or a null/zero
// result — callers get a runtime error, never a silent success.

#include <stddef.h>
#include <stdint.h>

// Keep in sync with rj_status.h. We deliberately don't include that
// header here — callers pick up the real types via the bindgen-
// generated Rust declarations, and this file only has to produce
// symbols the linker can resolve.
enum {
  RJ_STUB_STATUS_ERROR = 1,
};

/* ---- VM ----------------------------------------------------------- */

int rj_vm_create(const char *json_path, const char *schema_path, void **vm) {
  (void)json_path;
  (void)schema_path;
  if (vm) {
    *vm = NULL;
  }
  return RJ_STUB_STATUS_ERROR;
}

int rj_vm_create_from_string(const char *json, const char *schema_path, void **vm) {
  (void)json;
  (void)schema_path;
  if (vm) {
    *vm = NULL;
  }
  return RJ_STUB_STATUS_ERROR;
}

void rj_vm_retain(void *vm) { (void)vm; }
void rj_vm_release(void *vm) { (void)vm; }
void rj_vm_destroy(void *vm) { (void)vm; }

int rj_vm_step(void *vm, int *active) {
  (void)vm;
  if (active) {
    *active = 0;
  }
  return RJ_STUB_STATUS_ERROR;
}

int rj_vm_run(void *vm, uint64_t *ticks_executed) {
  (void)vm;
  if (ticks_executed) {
    *ticks_executed = 0;
  }
  return RJ_STUB_STATUS_ERROR;
}

int rj_vm_save_checkpoint(const void *vm, const char *path, uint64_t tick) {
  (void)vm;
  (void)path;
  (void)tick;
  return RJ_STUB_STATUS_ERROR;
}

int rj_vm_restore_checkpoint(const char *path, void **vm) {
  (void)path;
  if (vm) {
    *vm = NULL;
  }
  return RJ_STUB_STATUS_ERROR;
}

/* ---- Code: decoder ------------------------------------------------- */

int rj_code_decoder_create(int arch, void **decoder) {
  (void)arch;
  if (decoder) {
    *decoder = NULL;
  }
  return RJ_STUB_STATUS_ERROR;
}

void rj_code_decoder_retain(void *decoder) { (void)decoder; }
void rj_code_decoder_release(void *decoder) { (void)decoder; }
void rj_code_decoder_destroy(void *decoder) { (void)decoder; }

int rj_code_decoder_decode(void *decoder, const uint32_t *binary_inst, void **inst) {
  (void)decoder;
  (void)binary_inst;
  if (inst) {
    *inst = NULL;
  }
  return RJ_STUB_STATUS_ERROR;
}

/* ---- Code: executable --------------------------------------------- */

int rj_code_executable_create(const char *path, void **exec) {
  (void)path;
  if (exec) {
    *exec = NULL;
  }
  return RJ_STUB_STATUS_ERROR;
}

void rj_code_executable_retain(void *exec) { (void)exec; }
void rj_code_executable_release(void *exec) { (void)exec; }
void rj_code_executable_destroy(void *exec) { (void)exec; }

uint32_t rj_code_executable_num_code_objects(const void *exec, int target) {
  (void)exec;
  (void)target;
  return 0;
}

int rj_code_executable_get_code_object(const void *exec, int target, uint32_t index, void **obj) {
  (void)exec;
  (void)target;
  (void)index;
  if (obj) {
    *obj = NULL;
  }
  return RJ_STUB_STATUS_ERROR;
}

/* ---- Code: object -------------------------------------------------- */

void rj_code_object_retain(void *obj) { (void)obj; }
void rj_code_object_release(void *obj) { (void)obj; }
void rj_code_object_destroy(void *obj) { (void)obj; }

/* ---- Code: instruction list --------------------------------------- */

int rj_code_inst_list_create(void *obj, int target_id, void **inst_list) {
  (void)obj;
  (void)target_id;
  if (inst_list) {
    *inst_list = NULL;
  }
  return RJ_STUB_STATUS_ERROR;
}

void rj_code_inst_list_retain(void *inst_list) { (void)inst_list; }
void rj_code_inst_list_release(void *inst_list) { (void)inst_list; }
void rj_code_inst_list_destroy(void *inst_list) { (void)inst_list; }

/* ---- Code: basic block list --------------------------------------- */

int rj_code_basic_block_list_create(void *obj, int target_id, void **list) {
  (void)obj;
  (void)target_id;
  if (list) {
    *list = NULL;
  }
  return RJ_STUB_STATUS_ERROR;
}

void rj_code_basic_block_list_retain(void *list) { (void)list; }
void rj_code_basic_block_list_release(void *list) { (void)list; }
void rj_code_basic_block_list_destroy(void *list) { (void)list; }

uint32_t rj_code_basic_block_list_size(const void *list) {
  (void)list;
  return 0;
}

int rj_code_basic_block_list_get(const void *list, uint32_t index, void **block) {
  (void)list;
  (void)index;
  if (block) {
    *block = NULL;
  }
  return RJ_STUB_STATUS_ERROR;
}

/* ---- Code: basic block -------------------------------------------- */

void rj_code_basic_block_retain(void *block) { (void)block; }
void rj_code_basic_block_release(void *block) { (void)block; }
void rj_code_basic_block_destroy(void *block) { (void)block; }

uint64_t rj_code_basic_block_start_offset(const void *block) {
  (void)block;
  return 0;
}

uint32_t rj_code_basic_block_size(const void *block) {
  (void)block;
  return 0;
}

uint32_t rj_code_basic_block_num_instructions(const void *block) {
  (void)block;
  return 0;
}

const void *rj_code_basic_block_first_inst(const void *block) {
  (void)block;
  return NULL;
}

/* ---- Code: instruction -------------------------------------------- */

const char *rj_code_inst_mnemonic(const void *inst) {
  (void)inst;
  return "";
}

uint32_t rj_code_inst_size(const void *inst) {
  (void)inst;
  return 0;
}

uint32_t rj_code_inst_flags(const void *inst) {
  (void)inst;
  return 0;
}

int rj_code_inst_disassemble(const void *inst, char *buf, uint32_t buf_size) {
  (void)inst;
  if (buf && buf_size > 0) {
    buf[0] = '\0';
  }
  return RJ_STUB_STATUS_ERROR;
}

const void *rj_code_inst_next(const void *inst) {
  (void)inst;
  return NULL;
}

/* ---- KMD (simulated kernel-mode driver) ------------------------------- */

int rj_kmd_create_default(void **driver) {
  if (driver) {
    *driver = NULL;
  }
  return RJ_STUB_STATUS_ERROR;
}

int rj_kmd_create(const char *config_path, const char *schema_path, void **driver) {
  (void)config_path;
  (void)schema_path;
  if (driver) {
    *driver = NULL;
  }
  return RJ_STUB_STATUS_ERROR;
}

int rj_kmd_open(void *driver, int *fd) {
  (void)driver;
  if (fd) {
    *fd = -1;
  }
  return RJ_STUB_STATUS_ERROR;
}

int rj_kmd_close(void *driver) {
  (void)driver;
  return RJ_STUB_STATUS_ERROR;
}

int rj_kmd_ioctl(void *driver, unsigned long request, void *arg, int *result) {
  (void)driver;
  (void)request;
  (void)arg;
  if (result) {
    *result = -1;
  }
  return RJ_STUB_STATUS_ERROR;
}

int rj_kmd_mmap(void *driver, void *addr, size_t length, int prot, int flags,
                int64_t offset, void **result) {
  (void)driver;
  (void)addr;
  (void)length;
  (void)prot;
  (void)flags;
  (void)offset;
  if (result) {
    *result = (void *)(uintptr_t)-1; /* MAP_FAILED */
  }
  return RJ_STUB_STATUS_ERROR;
}

int rj_kmd_munmap(void *driver, void *addr, size_t length, int *result) {
  (void)driver;
  (void)addr;
  (void)length;
  if (result) {
    *result = -1;
  }
  return RJ_STUB_STATUS_ERROR;
}

int rj_kmd_fd(const void *driver) {
  (void)driver;
  return -1;
}

const char *rj_kmd_topology_path(const void *driver) {
  (void)driver;
  return NULL;
}

void rj_kmd_destroy(void *driver) { (void)driver; }
