/*************************************************************************
 * RCCL Bootstrap Tier-2 Deep Profiling — implementation
 *************************************************************************/

#include "bootstrap_trace.h"
#include "param.h"
#include "debug.h"

#include <atomic>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <pthread.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

namespace ncclBootstrapTrace {

NCCL_PARAM(BootstrapTrace, "BOOTSTRAP_TRACE", 0);

static std::atomic<int> s_initState{0};        // 0=uninit, 1=initing, 2=ready
static bool s_enabled = false;
static char s_outDir[256] = {0};

static void ensureGlobalInit() {
  int s = s_initState.load(std::memory_order_acquire);
  if (s == 2) return;
  int expected = 0;
  if (s_initState.compare_exchange_strong(expected, 1)) {
    s_enabled = ncclParamBootstrapTrace() != 0;
    const char* env = getenv("NCCL_BOOTSTRAP_TRACE_DIR");
    if (env && env[0]) {
      snprintf(s_outDir, sizeof(s_outDir), "%s", env);
    } else {
      snprintf(s_outDir, sizeof(s_outDir), "/tmp/bootstrap_trace");
    }
    if (s_enabled) {
      mkdir(s_outDir, 0755);
      INFO(NCCL_BOOTSTRAP, "BootstrapTrace: enabled, output dir: %s", s_outDir);
    }
    s_initState.store(2, std::memory_order_release);
  } else {
    while (s_initState.load(std::memory_order_acquire) != 2) {
      sched_yield();
    }
  }
}

bool isEnabled() {
  ensureGlobalInit();
  return s_enabled;
}

const char* outputDir() {
  ensureGlobalInit();
  return s_outDir;
}

static thread_local PerThreadBuffer* tl_buf = nullptr;

PerThreadBuffer* getBuffer() {
  if (tl_buf == nullptr && isEnabled()) {
    tl_buf = new PerThreadBuffer();
    tl_buf->idx = 0;
    tl_buf->rank = -1;
    tl_buf->isRootThread = 0;
  }
  return tl_buf;
}

void initThreadBuffer(int rank, int isRootThread) {
  if (!isEnabled()) return;
  PerThreadBuffer* b = getBuffer();
  if (!b) return;
  b->rank = rank;
  b->isRootThread = isRootThread;
  b->idx = 0;
}

void recordEvent(uint16_t phase, uint16_t md, uint64_t startNs, uint32_t bytes) {
  if (!isEnabled()) return;
  PerThreadBuffer* b = getBuffer();
  if (!b) return;
  if (b->idx >= RING_BUFFER_SIZE) return;
  uint64_t now = nowNs();
  Event& e = b->events[b->idx++];
  e.t_ns   = startNs;
  e.rank   = (uint32_t)b->rank;
  e.phase  = phase;
  e.md     = md;
  e.dur_us = (uint32_t)((now > startNs ? now - startNs : 0ULL) / 1000ULL);
  e.bytes  = bytes;
}

void recordInstant(uint16_t phase, uint16_t md) {
  if (!isEnabled()) return;
  PerThreadBuffer* b = getBuffer();
  if (!b) return;
  if (b->idx >= RING_BUFFER_SIZE) return;
  Event& e = b->events[b->idx++];
  e.t_ns   = nowNs();
  e.rank   = (uint32_t)b->rank;
  e.phase  = phase;
  e.md     = md;
  e.dur_us = 0;
  e.bytes  = 0;
}

void dumpThreadBuffer() {
  if (!isEnabled()) return;
  PerThreadBuffer* b = getBuffer();
  if (!b || b->idx == 0) return;

  char path[600];
  if (b->isRootThread) {
    snprintf(path, sizeof(path), "%s/root_pid%d_tid%lu.bin",
             s_outDir, (int)getpid(), (unsigned long)pthread_self());
  } else {
    snprintf(path, sizeof(path), "%s/rank%05d_pid%d.bin",
             s_outDir, b->rank, (int)getpid());
  }
  FILE* f = fopen(path, "wb");
  if (!f) {
    WARN("BootstrapTrace: cannot open %s for write", path);
    return;
  }
  // Header: magic(4), version(4), rank(4), isRoot(4), count(4), reserved(4)
  uint32_t hdr[6] = {
      0xB007F00D,
      1u,
      (uint32_t)b->rank,
      (uint32_t)b->isRootThread,
      (uint32_t)b->idx,
      0u};
  fwrite(hdr, sizeof(hdr), 1, f);
  fwrite(b->events, sizeof(Event), b->idx, f);
  fclose(f);
  // Reset idx after dump to avoid double-write if init is re-entered.
  b->idx = 0;
}

}  // namespace ncclBootstrapTrace
