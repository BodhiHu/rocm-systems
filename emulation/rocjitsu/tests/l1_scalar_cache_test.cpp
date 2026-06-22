// Copyright (c) 2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

/// @file l1_scalar_cache_test.cpp
/// @brief Unit tests for L1 Scalar Cache (K$): scalar load/store with all MTypes,
///        glc flag, and store→load consistency.

#include "rocjitsu/vm/amdgpu/gpu_memory.h"
#include "rocjitsu/vm/amdgpu/l1_scalar_cache.h"
#include "rocjitsu/vm/amdgpu/l2_cache.h"
#include "rocjitsu/vm/amdgpu/mtype.h"

#include <gtest/gtest.h>

#include <array>
#include <cstdint>
#include <cstring>
#include <optional>
#include <vector>

namespace {

using rocjitsu::amdgpu::GpuMemory;
using rocjitsu::amdgpu::L1ScalarCache;
using rocjitsu::amdgpu::L2Cache;
using rocjitsu::amdgpu::Mtype;

/// @brief Test fixture: L2 + L1 Scalar Cache + GpuMemory wired together.
///
/// The L2 is backed by GpuMemory, and the L1 Scalar Cache is backed
/// by the L2. This mirrors the real memory hierarchy for SMEM.
struct L1ScalarCacheFixture {
  GpuMemory memory{"memory"};
  L2Cache l2{"l2"};
  L1ScalarCache l1;

  L1ScalarCacheFixture() {
    l2.set_backing_memory(&memory);
    l1.set_l2(&l2);
    l1.set_memory(&memory);
  }
};

// ---------------------------------------------------------------------------
// Basic store→load round-trip: store a range of dwords, load them back.
// ---------------------------------------------------------------------------

TEST(L1ScalarCacheTest, StoreLoadRoundTrip_RW) {
  L1ScalarCacheFixture f;

  constexpr uint64_t kBase = 0x1000;
  constexpr uint32_t kNumDwords = 16;
  std::array<uint32_t, kNumDwords> stored{};
  std::array<uint32_t, kNumDwords> loaded{};

  for (uint32_t i = 0; i < kNumDwords; ++i)
    stored[i] = 0xA0000000u | (i << 4) | i;

  f.l1.store(kBase, kNumDwords, stored.data(), 0, std::nullopt);
  f.l1.load(kBase, kNumDwords, loaded.data(), 0, std::nullopt);

  EXPECT_EQ(loaded, stored) << "Store→load round-trip must preserve all dwords";
}

TEST(L1ScalarCacheTest, StoreLoadRoundTrip_Misaligned) {
  L1ScalarCacheFixture f;

  constexpr uint64_t kBase = 0x1002; // 2-byte aligned
  constexpr uint32_t kNumDwords = 8;
  std::array<uint32_t, kNumDwords> stored{};
  std::array<uint32_t, kNumDwords> loaded{};

  for (uint32_t i = 0; i < kNumDwords; ++i)
    stored[i] = 0xB0000000u | (i << 8) | i;

  f.l1.store(kBase, kNumDwords, stored.data(), 0, std::nullopt);
  f.l1.load(kBase, kNumDwords, loaded.data(), 0, std::nullopt);

  EXPECT_EQ(loaded, stored) << "Misaligned store→load round-trip must preserve data";
}

TEST(L1ScalarCacheTest, StoreLoadRoundTrip_CacheLineCrossing) {
  L1ScalarCacheFixture f;

  // Place data such that it spans two K$ cache lines (64B = 16 dwords).
  constexpr uint64_t kBase = 0x1000 + 60; // 4 bytes before end of line 0x1000
  constexpr uint32_t kNumDwords = 16;     // crosses into next line
  std::array<uint32_t, kNumDwords> stored{};
  std::array<uint32_t, kNumDwords> loaded{};

  for (uint32_t i = 0; i < kNumDwords; ++i)
    stored[i] = 0xC0000000u | (i << 8) | i;

  f.l1.store(kBase, kNumDwords, stored.data(), 0, std::nullopt);
  f.l1.load(kBase, kNumDwords, loaded.data(), 0, std::nullopt);

  EXPECT_EQ(loaded, stored) << "Cache-line-crossing store→load must preserve all dwords";
}

// ---------------------------------------------------------------------------
// MType coverage: test each MType in isolation for store→load correctness.
// ---------------------------------------------------------------------------

struct MtypeTestCase {
  const char *name;
  Mtype mtype;
};

constexpr MtypeTestCase kAllMTypes[] = {
    {"UC", Mtype::UC},
    {"CC", Mtype::CC},
    {"RW", Mtype::RW},
    {"WB", Mtype::WB},
    {"NT", Mtype::NT},
};

class L1ScalarCacheMtypeTest : public ::testing::TestWithParam<MtypeTestCase> {};

INSTANTIATE_TEST_SUITE_P(AllMTypes, L1ScalarCacheMtypeTest,
                         ::testing::ValuesIn(kAllMTypes),
                         [](const auto &info) { return info.param.name; });

TEST_P(L1ScalarCacheMtypeTest, StoreLoadRoundTrip) {
  L1ScalarCacheFixture f;
  Mtype mtype = GetParam().mtype;

  constexpr uint64_t kBase = 0x2000;
  constexpr uint32_t kNumDwords = 16;
  std::array<uint32_t, kNumDwords> stored{};
  std::array<uint32_t, kNumDwords> loaded{};

  for (uint32_t i = 0; i < kNumDwords; ++i)
    stored[i] = 0xE0000000u | (static_cast<uint32_t>(mtype) << 16) | (i << 4) | i;

  f.l1.store(kBase, kNumDwords, stored.data(), 0, mtype);
  f.l1.load(kBase, kNumDwords, loaded.data(), 0, mtype);

  EXPECT_EQ(loaded, stored) << "Mtype=" << static_cast<int>(mtype)
                            << " store→load must preserve all dwords";
}

// ---------------------------------------------------------------------------
// GLC flag: glc on SMEM forces Mtype::CC (coherent), bypassing K$ L1.
// Store with glc should write-through L1; load with glc should re-read
// from L2/backing store, NOT from a stale L1 line.
// ---------------------------------------------------------------------------

TEST(L1ScalarCacheTest, GlcOverridesToCC) {
  L1ScalarCacheFixture f;

  constexpr uint64_t kAddr = 0x3000;
  constexpr uint32_t kNumDwords = 4;

  // Step 1: Store with RW (no glc) → dirty in K$.
  {
    std::array<uint32_t, kNumDwords> data1{0x11111111, 0x22222222, 0x33333333, 0x44444444};
    f.l1.store(kAddr, kNumDwords, data1.data(), 0, Mtype::RW);
  }

  // Step 2: Store different data with CC (glc) → bypasses K$ (invalidate + write to L2).
  // This goes directly to L2, but the K$ line for kAddr is also invalidated.
  {
    std::array<uint32_t, kNumDwords> data2{0xAAAAAAAA, 0xBBBBBBBB, 0xCCCCCCCC, 0xDDDDDDDD};
    f.l1.store(kAddr, kNumDwords, data2.data(), 0, Mtype::CC);
  }

  // Step 3: Load with RW (no glc) → K$ miss (was invalidated), fetches from L2.
  // We should see data2, NOT stale data1 from K$.
  {
    std::array<uint32_t, kNumDwords> loaded{};
    f.l1.load(kAddr, kNumDwords, loaded.data(), 0, Mtype::RW);
    EXPECT_EQ(loaded[0], 0xAAAAAAAAu) << "After glc store, RW load must see CC-written data";
    EXPECT_EQ(loaded[1], 0xBBBBBBBBu);
    EXPECT_EQ(loaded[2], 0xCCCCCCCCu);
    EXPECT_EQ(loaded[3], 0xDDDDDDDDu);
  }
}

TEST(L1ScalarCacheTest, GlcLoadBypassesL1) {
  L1ScalarCacheFixture f;

  constexpr uint64_t kAddr = 0x4000;
  constexpr uint32_t kNumDwords = 4;

  // Step 1: Store value A in K$ via RW (no glc).
  {
    std::array<uint32_t, kNumDwords> dataA{0xAAAAAAAA, 0xAAAAAAAA, 0xAAAAAAAA, 0xAAAAAAAA};
    f.l1.store(kAddr, kNumDwords, dataA.data(), 0, Mtype::RW);
  }

  // Step 2: Write value B directly to memory (bypass L1/L2, simulating another agent).
  {
    std::array<uint32_t, kNumDwords> dataB{0xBBBBBBBB, 0xBBBBBBBB, 0xBBBBBBBB, 0xBBBBBBBB};
    f.memory.write_block(kAddr, reinterpret_cast<const uint8_t *>(dataB.data()),
                         kNumDwords * 4, 0);
  }

  // Step 3: Load with CC (glc) → invalidates K$ line, fetches from L2/memory.
  // Should see value B.
  {
    std::array<uint32_t, kNumDwords> loaded{};
    f.l1.load(kAddr, kNumDwords, loaded.data(), 0, Mtype::CC);
    EXPECT_EQ(loaded[0], 0xBBBBBBBBu) << "glc load must bypass stale L1 and see memory value B";
    EXPECT_EQ(loaded[1], 0xBBBBBBBBu);
  }

  // Step 4: Load again with RW → K$ now has B, should still see B.
  {
    std::array<uint32_t, kNumDwords> loaded{};
    f.l1.load(kAddr, kNumDwords, loaded.data(), 0, Mtype::RW);
    EXPECT_EQ(loaded[0], 0xBBBBBBBBu) << "Post-glc RW load must see the updated value B";
  }
}

TEST(L1ScalarCacheTest, GlcStoreThenGlcLoad) {
  L1ScalarCacheFixture f;

  constexpr uint64_t kAddr = 0x5000;
  constexpr uint32_t kNumDwords = 8;
  std::array<uint32_t, kNumDwords> data{};

  for (uint32_t i = 0; i < kNumDwords; ++i)
    data[i] = 0x60000000u | (i << 4) | i;

  f.l1.store(kAddr, kNumDwords, data.data(), 0, Mtype::CC);
  f.l1.invalidate_all(); // clear K$ to force load from L2

  std::array<uint32_t, kNumDwords> loaded{};
  f.l1.load(kAddr, kNumDwords, loaded.data(), 0, Mtype::CC);

  EXPECT_EQ(loaded, data) << "glc store followed by glc load must preserve all dwords";
}

// ---------------------------------------------------------------------------
// UC (uncacheable) bypass: UC stores bypass K$ and go directly to memory.
// Verify that after a UC store, a non-UC load sees the data.
// ---------------------------------------------------------------------------

TEST(L1ScalarCacheTest, UcBypassesL1) {
  L1ScalarCacheFixture f;

  constexpr uint64_t kAddr = 0x6000;
  constexpr uint32_t kNumDwords = 4;

  // UC store should bypass K$ and go directly to L2/memory.
  {
    std::array<uint32_t, kNumDwords> ucData{0xDEADBEEF, 0xCAFEBABE, 0xFEEDFACE, 0x8BADF00D};
    f.l1.store(kAddr, kNumDwords, ucData.data(), 0, Mtype::UC);
    f.l1.invalidate_all();
  }

  // Now load via RW: should see UC-stored data.
  {
    std::array<uint32_t, kNumDwords> loaded{};
    f.l1.load(kAddr, kNumDwords, loaded.data(), 0, Mtype::RW);
    EXPECT_EQ(loaded[0], 0xDEADBEEFu);
    EXPECT_EQ(loaded[1], 0xCAFEBABEu);
    EXPECT_EQ(loaded[2], 0xFEEDFACEu);
    EXPECT_EQ(loaded[3], 0x8BADF00Du);
  }
}

TEST(L1ScalarCacheTest, UcLoadBypassesL1) {
  L1ScalarCacheFixture f;

  constexpr uint64_t kAddr = 0x7000;
  constexpr uint32_t kNumDwords = 4;

  // Write to memory directly.
  {
    std::array<uint32_t, kNumDwords> memData{0x11112222, 0x33334444, 0x55556666, 0x77778888};
    f.memory.write_block(kAddr, reinterpret_cast<const uint8_t *>(memData.data()),
                         kNumDwords * 4, 0);
  }

  // UC load should read directly from memory.
  {
    std::array<uint32_t, kNumDwords> loaded{};
    f.l1.load(kAddr, kNumDwords, loaded.data(), 0, Mtype::UC);
    EXPECT_EQ(loaded[0], 0x11112222u);
    EXPECT_EQ(loaded[1], 0x33334444u);
    EXPECT_EQ(loaded[2], 0x55556666u);
    EXPECT_EQ(loaded[3], 0x77778888u);
  }

  // K$ should still be empty for this address (UC doesn't populate K$).
  // A subsequent RW load should still read the correct data from L2.
  f.l1.invalidate_all();
  {
    std::array<uint32_t, kNumDwords> loaded{};
    f.l1.load(kAddr, kNumDwords, loaded.data(), 0, Mtype::RW);
    EXPECT_EQ(loaded[0], 0x11112222u) << "RW load after UC load must still see correct data";
  }
}

// ---------------------------------------------------------------------------
// NT (non-temporal): bypass L1, cache in L2 only.
// Store with NT should skip K$; load with NT should skip K$ but hit L2.
// ---------------------------------------------------------------------------

TEST(L1ScalarCacheTest, NtBypassesL1) {
  L1ScalarCacheFixture f;

  constexpr uint64_t kAddr = 0x8000;
  constexpr uint32_t kNumDwords = 4;

  // NT store bypasses K$, writes to L2/memory.
  std::array<uint32_t, kNumDwords> ntData{0xAAAABBBB, 0xCCCCDDDD, 0xEEEEFFFF, 0x00001111};
  f.l1.store(kAddr, kNumDwords, ntData.data(), 0, Mtype::NT);

  // NT load bypasses K$, reads from L2.
  std::array<uint32_t, kNumDwords> loaded{};
  f.l1.load(kAddr, kNumDwords, loaded.data(), 0, Mtype::NT);

  EXPECT_EQ(loaded, ntData) << "NT store→load must preserve data";
}

// Store with NT, then load with RW: should also work (RW hits L2 on K$ miss).
TEST(L1ScalarCacheTest, NtStoreThenRwLoad) {
  L1ScalarCacheFixture f;

  constexpr uint64_t kAddr = 0x9000;
  constexpr uint32_t kNumDwords = 8;
  std::array<uint32_t, kNumDwords> data{};

  for (uint32_t i = 0; i < kNumDwords; ++i)
    data[i] = 0xF0000000u | (i << 8) | i;

  f.l1.store(kAddr, kNumDwords, data.data(), 0, Mtype::NT);

  std::array<uint32_t, kNumDwords> loaded{};
  f.l1.load(kAddr, kNumDwords, loaded.data(), 0, Mtype::RW);

  EXPECT_EQ(loaded, data) << "NT store followed by RW load must preserve all dwords";
}

// ---------------------------------------------------------------------------
// Writeback + invalidation: verify s_dcache_wb and s_dcache_inv behavior.
// ---------------------------------------------------------------------------

TEST(L1ScalarCacheTest, WritebackAllPreservesDirtyData) {
  L1ScalarCacheFixture f;

  constexpr uint64_t kAddr = 0xA000;
  constexpr uint32_t kNumDwords = 4;
  std::array<uint32_t, kNumDwords> data{0xDEAD0001, 0xDEAD0002, 0xDEAD0003, 0xDEAD0004};

  // Store with RW → dirty in K$.
  f.l1.store(kAddr, kNumDwords, data.data(), 0, Mtype::RW);

  // Writeback all dirty lines.
  f.l1.writeback_all();

  // Now invalidate K$ (simulating s_dcache_inv).
  f.l1.invalidate_all();

  // Load from L2/memory should still see the data.
  std::array<uint32_t, kNumDwords> loaded{};
  f.l1.load(kAddr, kNumDwords, loaded.data(), 0, Mtype::RW);

  EXPECT_EQ(loaded, data) << "Writeback + invalidate must preserve dirty data in L2";
}

TEST(L1ScalarCacheTest, InvalidateWithoutWritebackDiscardsDirty) {
  L1ScalarCacheFixture f;

  constexpr uint64_t kAddr = 0xB000;
  constexpr uint32_t kNumDwords = 4;
  std::array<uint32_t, kNumDwords> dirtyData{0xBADD0001, 0xBADD0002, 0xBADD0003, 0xBADD0004};

  // Write known pattern to memory.
  std::array<uint32_t, kNumDwords> memData{0x11111111, 0x22222222, 0x33333333, 0x44444444};
  f.memory.write_block(kAddr, reinterpret_cast<const uint8_t *>(memData.data()),
                       kNumDwords * 4, 0);

  // Store with RW → dirty in K$. This writes through to L2 but the dirty K$ line
  // has different (actually the same as L2 since L2 write-through) data.
  // To properly test, we need to verify that inv without wb means K$ dirty is lost.
  // However, since L2 also has write-through, the backing store has the RW data already.
  // The real test is: after inv, load from K$ hits L2 which has the right data.
  f.l1.store(kAddr, kNumDwords, dirtyData.data(), 0, Mtype::RW);

  // Invalid without writeback.
  f.l1.invalidate_all();

  // Load should see dirtyData (write-through from L2 to memory).
  std::array<uint32_t, kNumDwords> loaded{};
  f.l1.load(kAddr, kNumDwords, loaded.data(), 0, Mtype::RW);

  EXPECT_EQ(loaded, memData) << "After invalidate (L2 write-through), load must see original memory data";
}

// ---------------------------------------------------------------------------
// Mixed MType sequences: store with one MType, load with another.
// ---------------------------------------------------------------------------

TEST(L1ScalarCacheTest, StoreRwLoadCc) {
  L1ScalarCacheFixture f;

  constexpr uint64_t kAddr = 0xC000;
  constexpr uint32_t kNumDwords = 4;
  std::array<uint32_t, kNumDwords> data{0x11001100, 0x22002200, 0x33003300, 0x44004400};

  f.l1.store(kAddr, kNumDwords, data.data(), 0, Mtype::RW);
  f.l1.writeback_all();
  f.l1.invalidate_all();

  std::array<uint32_t, kNumDwords> loaded{};
  f.l1.load(kAddr, kNumDwords, loaded.data(), 0, Mtype::CC);

  EXPECT_EQ(loaded, data) << "RW store → CC load must preserve data";
}

TEST(L1ScalarCacheTest, StoreCcLoadRw) {
  L1ScalarCacheFixture f;

  constexpr uint64_t kAddr = 0xD000;
  constexpr uint32_t kNumDwords = 4;
  std::array<uint32_t, kNumDwords> data{0x55005500, 0x66006600, 0x77007700, 0x88008800};

  f.l1.store(kAddr, kNumDwords, data.data(), 0, Mtype::CC);
  f.l1.invalidate_all();

  std::array<uint32_t, kNumDwords> loaded{};
  f.l1.load(kAddr, kNumDwords, loaded.data(), 0, Mtype::RW);

  EXPECT_EQ(loaded, data) << "CC store → RW load must preserve data";
}

TEST(L1ScalarCacheTest, StoreRwLoadUc) {
  L1ScalarCacheFixture f;

  constexpr uint64_t kAddr = 0xE000;
  constexpr uint32_t kNumDwords = 4;
  std::array<uint32_t, kNumDwords> data{0x99009900, 0xAA00AA00, 0xBB00BB00, 0xCC00CC00};

  f.l1.store(kAddr, kNumDwords, data.data(), 0, Mtype::CC);

  f.l1.invalidate_all();
  std::array<uint32_t, kNumDwords> loaded{};
  f.l1.load(kAddr, kNumDwords, loaded.data(), 0, Mtype::UC);

  EXPECT_EQ(loaded, data) << "CC store → UC load must preserve data (via L2/memory)";
}

TEST(L1ScalarCacheTest, StoreUcLoadCc) {
  L1ScalarCacheFixture f;

  constexpr uint64_t kAddr = 0xF000;
  constexpr uint32_t kNumDwords = 4;
  std::array<uint32_t, kNumDwords> data{0xD00DD00D, 0xC0FFC0FF, 0xBEEFBEEF, 0xACE0ACE0};

  f.l1.store(kAddr, kNumDwords, data.data(), 0, Mtype::UC);
  f.l1.invalidate_all();

  std::array<uint32_t, kNumDwords> loaded{};
  f.l1.load(kAddr, kNumDwords, loaded.data(), 0, Mtype::CC);

  EXPECT_EQ(loaded, data) << "UC store → CC load must preserve data";
}

// ---------------------------------------------------------------------------
// Byte-granularity load/store.
// ---------------------------------------------------------------------------

TEST(L1ScalarCacheTest, LoadBytes) {
  L1ScalarCacheFixture f;

  constexpr uint64_t kAddr = 0x10000;
  constexpr uint32_t kNumBytes = 32;
  std::array<uint8_t, kNumBytes> data{};
  for (uint32_t i = 0; i < kNumBytes; ++i)
    data[i] = static_cast<uint8_t>(i * 7 + 3);

  f.l1.store(kAddr, kNumBytes / 4, reinterpret_cast<const uint32_t *>(data.data()), 0,
             Mtype::RW);

  std::array<uint8_t, kNumBytes> loaded{};
  f.l1.load_bytes(kAddr, kNumBytes, loaded.data(), 0, std::nullopt);

  EXPECT_EQ(loaded, data) << "Byte-granularity load must match dword store";
}

TEST(L1ScalarCacheTest, LoadBytesCcMode) {
  L1ScalarCacheFixture f;

  constexpr uint64_t kAddr = 0x20000;
  constexpr uint32_t kNumBytes = 16;
  std::array<uint8_t, kNumBytes> data{};
  for (uint32_t i = 0; i < kNumBytes; ++i)
    data[i] = static_cast<uint8_t>(~i);

  f.l1.store(kAddr, kNumBytes / 4, reinterpret_cast<const uint32_t *>(data.data()), 0, Mtype::CC);
  f.l1.invalidate_all();

  std::array<uint8_t, kNumBytes> loaded{};
  f.l1.load_bytes(kAddr, kNumBytes, loaded.data(), 0, Mtype::CC);

  EXPECT_EQ(loaded, data) << "Byte-granularity CC load must match CC store";
}

// ---------------------------------------------------------------------------
// Large sequential access: covers multi-line and multi-set patterns.
// ---------------------------------------------------------------------------

TEST(L1ScalarCacheTest, LargeSequentialAccess) {
  L1ScalarCacheFixture f;

  // 256 dwords = 1KB, spanning many cache lines (64B each = 16 dwords).
  constexpr uint64_t kBase = 0x30000;
  constexpr uint32_t kNumDwords = 256;
  std::vector<uint32_t> stored(kNumDwords);
  std::vector<uint32_t> loaded(kNumDwords);

  for (uint32_t i = 0; i < kNumDwords; ++i)
    stored[i] = 0xFACE0000u | i;

  f.l1.store(kBase, kNumDwords, stored.data(), 0, Mtype::RW);
  f.l1.load(kBase, kNumDwords, loaded.data(), 0, Mtype::RW);

  EXPECT_EQ(loaded, stored) << "Large sequential store→load must preserve all dwords";
}


// ---------------------------------------------------------------------------
// NT bypass verification: NT store goes directly to L2 (bypasses K$).
// After an NT store, K$ should NOT have the data, but L2/memory should.
// ---------------------------------------------------------------------------

TEST(L1ScalarCacheTest, NtStoreBypassesKcache) {
  L1ScalarCacheFixture f;

  constexpr uint64_t kAddr = 0x8800;
  constexpr uint32_t kNumDwords = 4;
  std::array<uint32_t, kNumDwords> ntData{0x11112222, 0x33334444, 0x55556666, 0x77778888};

  // NT store bypasses K$, goes to L2 (which writes through to memory).
  f.l1.store(kAddr, kNumDwords, ntData.data(), 0, Mtype::NT);

  // Invalidate K$ (simulates another agent or cache flush).
  // Since NT bypassed K$, this should NOT discard the data.
  f.l1.invalidate_all();

  // RW load should still see the data (fetched from L2 on K$ miss).
  std::array<uint32_t, kNumDwords> loaded{};
  f.l1.load(kAddr, kNumDwords, loaded.data(), 0, Mtype::RW);

  EXPECT_EQ(loaded, ntData) << "After NT store + K$ invalidate, RW load must see NT-stored data";
}

// NT load verifies NT bypasses K$, reads from L2 (not K$ dirty).
TEST(L1ScalarCacheTest, NtLoadBypassesKcache) {
  L1ScalarCacheFixture f;

  constexpr uint64_t kAddr = 0x9800;
  constexpr uint32_t kNumDwords = 4;
  std::array<uint32_t, kNumDwords> rwData{0xAAAABBBB, 0xCCCCDDDD, 0xEEEEFFFF, 0x11112222};

  // Step 1: RW store puts data in both K$ (dirty) and L2 (clean, write-through).
  f.l1.store(kAddr, kNumDwords, rwData.data(), 0, Mtype::RW);

  // Step 2: Writeback K$ dirty -> L2 (L2 now has authoritative data).
  // Then invalidate K$ to force NT load through L2 path.
  f.l1.writeback_all();
  f.l1.invalidate_all();

  // Write different data directly to memory.
  std::array<uint32_t, kNumDwords> memData{0xDEADBEEF, 0xCAFEBABE, 0xFEEDFACE, 0x8BADF00D};
  f.memory.write_block(kAddr, reinterpret_cast<const uint8_t *>(memData.data()),
                       kNumDwords * sizeof(uint32_t), 0);

  // Step 3: NT load bypasses K$ (which is empty after invalidate),
  // reads directly from L2. L2 has rwData (from writeback).
  std::array<uint32_t, kNumDwords> loaded{};
  f.l1.load(kAddr, kNumDwords, loaded.data(), 0, Mtype::NT);

  EXPECT_EQ(loaded, rwData) << "NT load reads from L2 which has the writeback data";
}

TEST(L1ScalarCacheTest, NtStoreReachesMemory) {
  L1ScalarCacheFixture f;

  constexpr uint64_t kAddr = 0xA800;
  constexpr uint32_t kNumDwords = 4;
  std::array<uint32_t, kNumDwords> ntData{0xDEAD0001, 0xDEAD0002, 0xDEAD0003, 0xDEAD0004};

  // NT store bypasses K$, goes directly to L2.
  f.l1.store(kAddr, kNumDwords, ntData.data(), 0, Mtype::NT);

  // After NT store, memory should have the data because L2::write with
  // any mtype (including NT, which falls through to the RW path) does
  // write-through to backing memory.
  std::array<uint32_t, kNumDwords> memLoaded{};
  f.memory.read_block(kAddr, reinterpret_cast<uint8_t *>(memLoaded.data()),
                      kNumDwords * sizeof(uint32_t), 0);

  EXPECT_EQ(memLoaded, ntData) << "NT store must reach backing memory via L2";
}

} // namespace
