#pragma once

#include "rocjitsu/code/rj_code.h"
#include "rocjitsu/vm/rj_vm.h"

#ifndef __cplusplus
#error "rocjitsu VM plugins use the C++ plugin API"
#endif

#define RJ_VM_PLUGIN_ENTRYPOINT "get_rj_vm_plugin"


struct rj_vm_plugin_config_args_t {
  const char *config;
};

/// Called once when the plugin is attached.
struct rj_vm_plugin_init_args_t {
  uint32_t reserved;
};

/// Called before every AMDGPU instruction is executed.
struct rj_vm_plugin_amdgpu_execute_instruction_args_t {
  uint64_t pc;
  const rj_code_inst_state_t *inst;
};

/// Called when an AMDGPU memory instruction is routed to a pipeline.
struct rj_vm_plugin_amdgpu_route_memory_instruction_args_t {
  const rj_code_inst_state_t *inst;
};

/// Called when a new AMDGPU kernel dispatch begins.
struct rj_vm_plugin_amdgpu_kernel_dispatch_args_t {
  uint64_t kernel_object;
  uint64_t entry_pc;
};

/// Called after a workgroup's wavefronts have been dispatched to a CU.
struct rj_vm_plugin_amdgpu_dispatch_workgroup_args_t {
  uint32_t wg_id;
  uint32_t vgpr_count;
  uint32_t sgpr_count;
  rocjitsu::amdgpu::Wavefront *const *wavefronts;
  uint32_t wavefront_count;
};

/// Called when a VGPR is read during instruction execution.
/// Not yet wired - to be connected when race detection lands.
struct rj_vm_plugin_amdgpu_read_vgpr_args_t {
  rocjitsu::amdgpu::Wavefront *wf;
  uint32_t logical_reg;
  uint32_t lane;
};

/// Called when an SGPR is read during instruction execution.
/// Not yet wired - to be connected when race detection lands.
struct rj_vm_plugin_amdgpu_read_sgpr_args_t {
  rocjitsu::amdgpu::Wavefront *wf;
  uint32_t logical_reg;
};

/// Called when s_waitcnt sets counter thresholds.
struct rj_vm_plugin_amdgpu_set_wait_target_args_t {
  rocjitsu::amdgpu::Wavefront *wf;
  int vmcnt;
  int lgkmcnt;
};

/// Called when all waves in a workgroup have reached s_barrier.
struct rj_vm_plugin_amdgpu_barrier_resolved_args_t {
  uint32_t wg_id;
};

// -- RISC-V hooks --------------------------------------------------------

/// Called before every RISC-V instruction is executed.
struct rj_vm_plugin_riscv_execute_instruction_args_t {
  uint64_t pc;
  const rj_code_inst_state_t *inst;
};

/// Called once before the plugin is detached.
struct rj_vm_plugin_cleanup_args_t {
  uint32_t reserved;
};

/// Called with JSON-formatted config options declared in the plugin's manifest file.
/// Return 0 on success, non-zero on failure (e.g. invalid config).
typedef int (*rj_vm_plugin_config_fn)(const rj_vm_plugin_config_args_t *args);
typedef int (*rj_vm_plugin_init_fn)(const rj_vm_plugin_init_args_t *args);
typedef void (*rj_vm_plugin_amdgpu_execute_instruction_fn)(
    const rj_vm_plugin_amdgpu_execute_instruction_args_t *args);
typedef void (*rj_vm_plugin_amdgpu_route_memory_instruction_fn)(
    const rj_vm_plugin_amdgpu_route_memory_instruction_args_t *args);
typedef void (*rj_vm_plugin_amdgpu_kernel_dispatch_fn)(
    const rj_vm_plugin_amdgpu_kernel_dispatch_args_t *args);
typedef void (*rj_vm_plugin_amdgpu_dispatch_workgroup_fn)(
    const rj_vm_plugin_amdgpu_dispatch_workgroup_args_t *args);
typedef void (*rj_vm_plugin_amdgpu_read_vgpr_fn)(const rj_vm_plugin_amdgpu_read_vgpr_args_t *args);
typedef void (*rj_vm_plugin_amdgpu_read_sgpr_fn)(const rj_vm_plugin_amdgpu_read_sgpr_args_t *args);
typedef void (*rj_vm_plugin_amdgpu_set_wait_target_fn)(
    const rj_vm_plugin_amdgpu_set_wait_target_args_t *args);
typedef void (*rj_vm_plugin_amdgpu_barrier_resolved_fn)(
    const rj_vm_plugin_amdgpu_barrier_resolved_args_t *args);
typedef void (*rj_vm_plugin_riscv_execute_instruction_fn)(
    const rj_vm_plugin_riscv_execute_instruction_args_t *args);
typedef void (*rj_vm_plugin_cleanup_fn)(const rj_vm_plugin_cleanup_args_t *args);

struct rj_vm_plugin_t {
  rj_vm_plugin_config_fn config;
  rj_vm_plugin_init_fn init;
  rj_vm_plugin_amdgpu_execute_instruction_fn on_amdgpu_execute_instruction;
  rj_vm_plugin_amdgpu_route_memory_instruction_fn on_amdgpu_route_memory_instruction;
  rj_vm_plugin_amdgpu_kernel_dispatch_fn on_amdgpu_kernel_dispatch;
  rj_vm_plugin_amdgpu_dispatch_workgroup_fn on_amdgpu_dispatch_workgroup;
  rj_vm_plugin_amdgpu_read_vgpr_fn on_amdgpu_read_vgpr;
  rj_vm_plugin_amdgpu_read_sgpr_fn on_amdgpu_read_sgpr;
  rj_vm_plugin_amdgpu_set_wait_target_fn on_amdgpu_set_wait_target;
  rj_vm_plugin_amdgpu_barrier_resolved_fn on_amdgpu_barrier_resolved;
  rj_vm_plugin_riscv_execute_instruction_fn on_riscv_execute_instruction;
  rj_vm_plugin_cleanup_fn cleanup;
};

typedef rj_vm_plugin_t (*get_rj_vm_plugin_fn)(void);

extern "C" {
RJ_API_EXPORT rj_vm_plugin_t get_rj_vm_plugin(void);
} // extern "C"