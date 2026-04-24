// Copyright (c) 2025-2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

#pragma once

#include <cstdint>
#include <string>
#include <vector>

#include "rocjitsu/code/rj_code.h"

namespace rocjitsu {

class AmdGpuCodeObject;

struct TranslatedCodeObject {
  std::vector<uint8_t> elf_bytes;
  rj_code_arch_t host_arch;
  std::vector<std::string> warnings;
};

class BinaryTranslator {
public:
  BinaryTranslator(rj_code_arch_t guest_arch, rj_code_arch_t host_arch);

  TranslatedCodeObject translate(const AmdGpuCodeObject &obj);

private:
  rj_code_arch_t guest_arch_;
  rj_code_arch_t host_arch_;
};

} // namespace rocjitsu
