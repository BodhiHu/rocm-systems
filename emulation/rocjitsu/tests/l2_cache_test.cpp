// Copyright (c) 2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

#include "simdojo/components/sparse_memory.h"

#include <gtest/gtest.h>

#include <array>
#include <barrier>
#include <cstdint>
#include <cstring>
#include <thread>
#include <vector>

namespace {

TEST(SparseMemoryThreadingTest, ConcurrentDifferentPageWritesArePreserved) {
  simdojo::SparseMemory memory("memory");

  constexpr uint32_t kThreads = 8;
  constexpr uint32_t kPagesPerThread = 64;
  constexpr uint64_t kBase = 0x800000;

  std::barrier start(kThreads);
  std::vector<std::thread> workers;
  workers.reserve(kThreads);

  for (uint32_t tid = 0; tid < kThreads; ++tid) {
    workers.emplace_back([&, tid] {
      std::array<uint8_t, simdojo::SparseMemory::PAGE_SIZE> page{};
      start.arrive_and_wait();
      for (uint32_t i = 0; i < kPagesPerThread; ++i) {
        const uint64_t addr =
            kBase + (static_cast<uint64_t>(i) * kThreads + tid) * simdojo::SparseMemory::PAGE_SIZE;
        for (uint32_t b = 0; b < page.size(); ++b)
          page[b] = static_cast<uint8_t>((tid << 4) ^ i ^ b);
        memory.write_block(addr, page.data(), page.size());
      }
    });
  }

  for (auto &worker : workers)
    worker.join();

  EXPECT_EQ(memory.num_pages(), kThreads * kPagesPerThread);

  std::array<uint8_t, simdojo::SparseMemory::PAGE_SIZE> expected{};
  std::array<uint8_t, simdojo::SparseMemory::PAGE_SIZE> actual{};
  for (uint32_t tid = 0; tid < kThreads; ++tid) {
    for (uint32_t i = 0; i < kPagesPerThread; ++i) {
      const uint64_t addr =
          kBase + (static_cast<uint64_t>(i) * kThreads + tid) * simdojo::SparseMemory::PAGE_SIZE;
      for (uint32_t b = 0; b < expected.size(); ++b)
        expected[b] = static_cast<uint8_t>((tid << 4) ^ i ^ b);
      memory.read_block(addr, actual.data(), actual.size());
      EXPECT_EQ(actual, expected) << "addr=0x" << std::hex << addr;
      uint32_t expected_word = 0;
      std::memcpy(&expected_word, expected.data(), sizeof(expected_word));
      EXPECT_EQ(memory.read32(addr), expected_word);
    }
  }
}

} // namespace
