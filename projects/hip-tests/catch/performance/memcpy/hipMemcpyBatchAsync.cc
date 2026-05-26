/*
 * Copyright (c) Advanced Micro Devices, Inc., or its affiliates.
 *
 * SPDX-License-Identifier: MIT
 */

#include "memcpy_performance_common.hh"

#include <cstring>
#include <string>
#include <tuple>
#include <vector>

/**
 * @addtogroup memcpy memcpy
 * @{
 * @ingroup PerformanceTest
 */

namespace {

constexpr unsigned char kPattern = 0x5a;

enum class PointerPattern {
  BasePointers,
  BroadcastSource,
  UnalignedPointers,
};

std::string GetSizeSectionName(size_t size) {
  if (size < 1_MB) {
    return std::to_string(size / 1_KB) + " KB";
  }
  return std::to_string(size / 1_MB) + " MB";
}

std::string GetPointerPatternSectionName(PointerPattern pointer_pattern) {
  switch (pointer_pattern) {
  case PointerPattern::BasePointers:
    return "base pointers";
  case PointerPattern::BroadcastSource:
    return "broadcast source";
  case PointerPattern::UnalignedPointers:
    return "unaligned pointers";
  }
  return "unknown pointer pattern";
}

size_t AlignUp(size_t value, size_t alignment) {
  return ((value + alignment - 1) / alignment) * alignment;
}

class MemcpyBatchAsync : public Benchmark<MemcpyBatchAsync> {
public:
  void operator()(void **dsts, void **srcs, size_t *sizes, size_t count,
                  bool prefer_ce) {
    size_t attrs_idxs[1] = {0};
    hipMemcpyAttributes attr{};
    attr.srcAccessOrder = hipMemcpySrcAccessOrderStream;
    attr.flags = hipMemcpyFlagExtPreferCE;
    hipMemcpyAttributes *attrs = prefer_ce ? &attr : nullptr;
    size_t *attr_indices = prefer_ce ? attrs_idxs : nullptr;
    const size_t num_attrs = prefer_ce ? 1 : 0;

    TIMED_SECTION(kTimerTypeCpu) {
      HIP_CHECK(hipMemcpyBatchAsync(dsts, srcs, sizes, count, attrs,
                                    attr_indices, num_attrs, nullptr, nullptr));
    }
  }
};

void RunDeviceToDeviceBenchmark(size_t copy_size, size_t batch_copy_count,
                                size_t allocation_alignment, bool prefer_ce,
                                PointerPattern pointer_pattern) {
  MemcpyBatchAsync benchmark;
  benchmark.AddSectionName(std::to_string(allocation_alignment) +
                           "-byte aligned");
  benchmark.AddSectionName(GetPointerPatternSectionName(pointer_pattern));
  benchmark.AddSectionName(GetSizeSectionName(copy_size));
  benchmark.AddSectionName(std::to_string(batch_copy_count) + " copies");
  benchmark.RegisterBandwidth(copy_size * batch_copy_count);

  const size_t stride = AlignUp(copy_size, allocation_alignment);
  const size_t offset_bytes =
      pointer_pattern == PointerPattern::UnalignedPointers ? 1 : 0;
  const size_t allocation_size = offset_bytes + (stride * batch_copy_count);

  LinearAllocGuard<unsigned char> src_allocation(LinearAllocs::hipMalloc,
                                                 allocation_size);
  LinearAllocGuard<unsigned char> dst_allocation(LinearAllocs::hipMalloc,
                                                 allocation_size);
  HIP_CHECK(hipMemset(src_allocation.ptr(), kPattern, allocation_size));

  std::vector<void *> srcs(batch_copy_count);
  std::vector<void *> dsts(batch_copy_count);
  std::vector<size_t> sizes(batch_copy_count, copy_size);
  for (size_t i = 0; i < batch_copy_count; ++i) {
    srcs[i] = pointer_pattern == PointerPattern::BroadcastSource
                  ? src_allocation.ptr() + offset_bytes
                  : src_allocation.ptr() + offset_bytes + (i * stride);
    dsts[i] = dst_allocation.ptr() + offset_bytes + (i * stride);
  }

  benchmark.Run(dsts.data(), srcs.data(), sizes.data(), sizes.size(),
                prefer_ce);
}

void RunPeerToPeerBenchmark(size_t copy_size, size_t batch_copy_count,
                            size_t allocation_alignment, bool prefer_ce,
                            PointerPattern pointer_pattern) {
  MemcpyBatchAsync benchmark;
  benchmark.AddSectionName(std::to_string(allocation_alignment) +
                           "-byte aligned");
  benchmark.AddSectionName(GetPointerPatternSectionName(pointer_pattern));
  benchmark.AddSectionName(GetSizeSectionName(copy_size));
  benchmark.AddSectionName(std::to_string(batch_copy_count) + " copies");
  benchmark.RegisterBandwidth(copy_size * batch_copy_count);

  const size_t stride = AlignUp(copy_size, allocation_alignment);
  const size_t offset_bytes =
      pointer_pattern == PointerPattern::UnalignedPointers ? 1 : 0;
  const size_t allocation_size = offset_bytes + (stride * batch_copy_count);
  HIP_CHECK(hipSetDevice(0));
  auto [src_device, dst_device] = GetDeviceIds(true);

  HIP_CHECK(hipSetDevice(src_device));
  LinearAllocGuard<unsigned char> src_allocation(LinearAllocs::hipMalloc,
                                                 allocation_size);
  HIP_CHECK(hipMemset(src_allocation.ptr(), kPattern, allocation_size));

  HIP_CHECK(hipSetDevice(dst_device));
  LinearAllocGuard<unsigned char> dst_allocation(LinearAllocs::hipMalloc,
                                                 allocation_size);

  std::vector<void *> srcs(batch_copy_count);
  std::vector<void *> dsts(batch_copy_count);
  std::vector<size_t> sizes(batch_copy_count, copy_size);
  for (size_t i = 0; i < batch_copy_count; ++i) {
    srcs[i] = pointer_pattern == PointerPattern::BroadcastSource
                  ? src_allocation.ptr() + offset_bytes
                  : src_allocation.ptr() + offset_bytes + (i * stride);
    dsts[i] = dst_allocation.ptr() + offset_bytes + (i * stride);
  }

  HIP_CHECK(hipSetDevice(src_device));
  benchmark.Run(dsts.data(), srcs.data(), sizes.data(), sizes.size(),
                prefer_ce);
}

void RunHostDeviceBenchmark(size_t copy_size, size_t batch_copy_count,
                            size_t allocation_alignment,
                            LinearAllocs src_allocation_type,
                            LinearAllocs dst_allocation_type, bool prefer_ce,
                            PointerPattern pointer_pattern) {
  MemcpyBatchAsync benchmark;
  benchmark.AddSectionName(std::to_string(allocation_alignment) +
                           "-byte aligned");
  benchmark.AddSectionName(GetPointerPatternSectionName(pointer_pattern));
  benchmark.AddSectionName(GetSizeSectionName(copy_size));
  benchmark.AddSectionName(std::to_string(batch_copy_count) + " copies");
  benchmark.RegisterBandwidth(copy_size * batch_copy_count);

  const size_t stride = AlignUp(copy_size, allocation_alignment);
  const size_t offset_bytes =
      pointer_pattern == PointerPattern::UnalignedPointers ? 1 : 0;
  const size_t allocation_size = offset_bytes + (stride * batch_copy_count);

  LinearAllocGuard<unsigned char> src_allocation(src_allocation_type,
                                                 allocation_size);
  LinearAllocGuard<unsigned char> dst_allocation(dst_allocation_type,
                                                 allocation_size);
  if (src_allocation_type == LinearAllocs::hipMalloc) {
    HIP_CHECK(hipMemset(src_allocation.ptr(), kPattern, allocation_size));
  } else {
    std::memset(src_allocation.host_ptr(), kPattern, allocation_size);
  }

  std::vector<void *> srcs(batch_copy_count);
  std::vector<void *> dsts(batch_copy_count);
  std::vector<size_t> sizes(batch_copy_count, copy_size);
  for (size_t i = 0; i < batch_copy_count; ++i) {
    srcs[i] = pointer_pattern == PointerPattern::BroadcastSource
                  ? src_allocation.ptr() + offset_bytes
                  : src_allocation.ptr() + offset_bytes + (i * stride);
    dsts[i] = dst_allocation.ptr() + offset_bytes + (i * stride);
  }

  benchmark.Run(dsts.data(), srcs.data(), sizes.data(), sizes.size(),
                prefer_ce);
}

} // namespace

HIP_TEST_CASE(Performance_hipMemcpyBatchAsync_D2D) {
  const size_t allocation_alignment = GENERATE(64, 128);
  const PointerPattern pointer_pattern =
      GENERATE(PointerPattern::BasePointers, PointerPattern::BroadcastSource,
               PointerPattern::UnalignedPointers);
  const auto [copy_size, batch_copy_count] = GENERATE(table<size_t, size_t>(
      {{4_KB, 1024}, {16_KB, 256}, {64_KB, 64}, {256_KB, 16}, {1024_KB, 4}}));
  RunDeviceToDeviceBenchmark(copy_size, batch_copy_count, allocation_alignment,
                             false, pointer_pattern);
}

HIP_TEST_CASE(Performance_hipMemcpyBatchAsync_D2D_PreferCE) {
  const size_t allocation_alignment = GENERATE(64, 128);
  const PointerPattern pointer_pattern =
      GENERATE(PointerPattern::BasePointers, PointerPattern::BroadcastSource,
               PointerPattern::UnalignedPointers);
  const auto [copy_size, batch_copy_count] = GENERATE(table<size_t, size_t>(
      {{4_KB, 1024}, {16_KB, 256}, {64_KB, 64}, {256_KB, 16}, {1024_KB, 4}}));
  RunDeviceToDeviceBenchmark(copy_size, batch_copy_count, allocation_alignment,
                             true, pointer_pattern);
}

HIP_TEST_CASE(Performance_hipMemcpyBatchAsync_P2P) {
  if (HipTest::getDeviceCount() < 2) {
    HIP_SKIP_TEST(HipTest::SkipReason::kFewerThanTwoGpus);
  }
  const size_t allocation_alignment = GENERATE(64, 128);
  const PointerPattern pointer_pattern =
      GENERATE(PointerPattern::BasePointers, PointerPattern::BroadcastSource,
               PointerPattern::UnalignedPointers);
  const auto [copy_size, batch_copy_count] = GENERATE(table<size_t, size_t>(
      {{4_KB, 1024}, {16_KB, 256}, {64_KB, 64}, {256_KB, 16}, {1024_KB, 4}}));
  RunPeerToPeerBenchmark(copy_size, batch_copy_count, allocation_alignment,
                         false, pointer_pattern);
}

HIP_TEST_CASE(Performance_hipMemcpyBatchAsync_P2P_PreferCE) {
  if (HipTest::getDeviceCount() < 2) {
    HIP_SKIP_TEST(HipTest::SkipReason::kFewerThanTwoGpus);
  }
  const size_t allocation_alignment = GENERATE(64, 128);
  const PointerPattern pointer_pattern =
      GENERATE(PointerPattern::BasePointers, PointerPattern::BroadcastSource,
               PointerPattern::UnalignedPointers);
  const auto [copy_size, batch_copy_count] = GENERATE(table<size_t, size_t>(
      {{4_KB, 1024}, {16_KB, 256}, {64_KB, 64}, {256_KB, 16}, {1024_KB, 4}}));
  RunPeerToPeerBenchmark(copy_size, batch_copy_count, allocation_alignment,
                         true, pointer_pattern);
}

HIP_TEST_CASE(Performance_hipMemcpyBatchAsync_H2D_Pageable) {
  const size_t allocation_alignment = GENERATE(64, 128);
  const PointerPattern pointer_pattern =
      GENERATE(PointerPattern::BasePointers, PointerPattern::BroadcastSource,
               PointerPattern::UnalignedPointers);
  const auto [copy_size, batch_copy_count] = GENERATE(table<size_t, size_t>(
      {{4_KB, 1024}, {16_KB, 256}, {64_KB, 64}, {256_KB, 16}, {1024_KB, 4}}));
  RunHostDeviceBenchmark(copy_size, batch_copy_count, allocation_alignment,
                         LinearAllocs::malloc, LinearAllocs::hipMalloc, false,
                         pointer_pattern);
}

HIP_TEST_CASE(Performance_hipMemcpyBatchAsync_H2D_Pinned) {
  const size_t allocation_alignment = GENERATE(64, 128);
  const PointerPattern pointer_pattern =
      GENERATE(PointerPattern::BasePointers, PointerPattern::BroadcastSource,
               PointerPattern::UnalignedPointers);
  const auto [copy_size, batch_copy_count] = GENERATE(table<size_t, size_t>(
      {{4_KB, 1024}, {16_KB, 256}, {64_KB, 64}, {256_KB, 16}, {1024_KB, 4}}));
  RunHostDeviceBenchmark(copy_size, batch_copy_count, allocation_alignment,
                         LinearAllocs::hipHostMalloc, LinearAllocs::hipMalloc,
                         false, pointer_pattern);
}

HIP_TEST_CASE(Performance_hipMemcpyBatchAsync_H2D_PreferCE_Pageable) {
  const size_t allocation_alignment = GENERATE(64, 128);
  const PointerPattern pointer_pattern =
      GENERATE(PointerPattern::BasePointers, PointerPattern::BroadcastSource,
               PointerPattern::UnalignedPointers);
  const auto [copy_size, batch_copy_count] = GENERATE(table<size_t, size_t>(
      {{4_KB, 1024}, {16_KB, 256}, {64_KB, 64}, {256_KB, 16}, {1024_KB, 4}}));
  RunHostDeviceBenchmark(copy_size, batch_copy_count, allocation_alignment,
                         LinearAllocs::malloc, LinearAllocs::hipMalloc, true,
                         pointer_pattern);
}

HIP_TEST_CASE(Performance_hipMemcpyBatchAsync_H2D_PreferCE_Pinned) {
  const size_t allocation_alignment = GENERATE(64, 128);
  const PointerPattern pointer_pattern =
      GENERATE(PointerPattern::BasePointers, PointerPattern::BroadcastSource,
               PointerPattern::UnalignedPointers);
  const auto [copy_size, batch_copy_count] = GENERATE(table<size_t, size_t>(
      {{4_KB, 1024}, {16_KB, 256}, {64_KB, 64}, {256_KB, 16}, {1024_KB, 4}}));
  RunHostDeviceBenchmark(copy_size, batch_copy_count, allocation_alignment,
                         LinearAllocs::hipHostMalloc, LinearAllocs::hipMalloc,
                         true, pointer_pattern);
}

HIP_TEST_CASE(Performance_hipMemcpyBatchAsync_D2H_Pageable) {
  const size_t allocation_alignment = GENERATE(64, 128);
  const PointerPattern pointer_pattern =
      GENERATE(PointerPattern::BasePointers, PointerPattern::BroadcastSource,
               PointerPattern::UnalignedPointers);
  const auto [copy_size, batch_copy_count] = GENERATE(table<size_t, size_t>(
      {{4_KB, 1024}, {16_KB, 256}, {64_KB, 64}, {256_KB, 16}, {1024_KB, 4}}));
  RunHostDeviceBenchmark(copy_size, batch_copy_count, allocation_alignment,
                         LinearAllocs::hipMalloc, LinearAllocs::malloc, false,
                         pointer_pattern);
}

HIP_TEST_CASE(Performance_hipMemcpyBatchAsync_D2H_Pinned) {
  const size_t allocation_alignment = GENERATE(64, 128);
  const PointerPattern pointer_pattern =
      GENERATE(PointerPattern::BasePointers, PointerPattern::BroadcastSource,
               PointerPattern::UnalignedPointers);
  const auto [copy_size, batch_copy_count] = GENERATE(table<size_t, size_t>(
      {{4_KB, 1024}, {16_KB, 256}, {64_KB, 64}, {256_KB, 16}, {1024_KB, 4}}));
  RunHostDeviceBenchmark(copy_size, batch_copy_count, allocation_alignment,
                         LinearAllocs::hipMalloc, LinearAllocs::hipHostMalloc,
                         false, pointer_pattern);
}

HIP_TEST_CASE(Performance_hipMemcpyBatchAsync_D2H_PreferCE_Pageable) {
  const size_t allocation_alignment = GENERATE(64, 128);
  const PointerPattern pointer_pattern =
      GENERATE(PointerPattern::BasePointers, PointerPattern::BroadcastSource,
               PointerPattern::UnalignedPointers);
  const auto [copy_size, batch_copy_count] = GENERATE(table<size_t, size_t>(
      {{4_KB, 1024}, {16_KB, 256}, {64_KB, 64}, {256_KB, 16}, {1024_KB, 4}}));
  RunHostDeviceBenchmark(copy_size, batch_copy_count, allocation_alignment,
                         LinearAllocs::hipMalloc, LinearAllocs::malloc, true,
                         pointer_pattern);
}

HIP_TEST_CASE(Performance_hipMemcpyBatchAsync_D2H_PreferCE_Pinned) {
  const size_t allocation_alignment = GENERATE(64, 128);
  const PointerPattern pointer_pattern =
      GENERATE(PointerPattern::BasePointers, PointerPattern::BroadcastSource,
               PointerPattern::UnalignedPointers);
  const auto [copy_size, batch_copy_count] = GENERATE(table<size_t, size_t>(
      {{4_KB, 1024}, {16_KB, 256}, {64_KB, 64}, {256_KB, 16}, {1024_KB, 4}}));
  RunHostDeviceBenchmark(copy_size, batch_copy_count, allocation_alignment,
                         LinearAllocs::hipMalloc, LinearAllocs::hipHostMalloc,
                         true, pointer_pattern);
}

/**
 * End doxygen group memcpy.
 * @}
 */
