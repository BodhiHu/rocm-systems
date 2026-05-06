# Copyright (c) 2026 Advanced Micro Devices, Inc.
# SPDX-License-Identifier: MIT

include(rj_configure_target)

add_executable(rocjitsu-daemon main.cpp)

target_link_libraries(rocjitsu-daemon PRIVATE
    rocjitsu_kmd
    rocjitsu_code rocjitsu_isa
    rocjitsu_isa_cdna1 rocjitsu_isa_cdna2 rocjitsu_isa_cdna3 rocjitsu_isa_cdna4
    rocjitsu_isa_rdna1 rocjitsu_isa_rdna2 rocjitsu_isa_rdna3 rocjitsu_isa_rdna3_5 rocjitsu_isa_rdna4
    rocjitsu_isa_risc_v
    simdojo rocjitsu_vm rocjitsu_vm_amdgpu rocjitsu_vm_risc_v rocjitsu_config
    util flatbuffers Threads::Threads dl)

# LivenessAnalysis symbols are missing on develop (pre-existing).
# The daemon doesn't use DBT translation, so these are dead code references
# pulled in transitively through rocjitsu_code.
target_link_options(rocjitsu-daemon PRIVATE "LINKER:--unresolved-symbols=ignore-in-object-files")

rj_configure_target(rocjitsu-daemon TOOL)
