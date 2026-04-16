// ioctl_dispatch.cpp — Serializes ioctl calls to FlatBuffer RPC messages
#include "ioctl_dispatch.h"
#include "../rpc/client.h"
#include "rpc_generated.h"
#include "rpc/str_util.h"

#include <cerrno>
#include <cstring>
#include <dlfcn.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <unistd.h>

// DRM ioctl definitions (matching kernel UAPI)
#define DRM_COMMAND_BASE 0x40
#define DRM_IOCTL_BASE 'd'

// AMDGPU DRM ioctl offsets
#define DRM_AMDGPU_GEM_CREATE       0x00
#define DRM_AMDGPU_GEM_MMAP         0x01
#define DRM_AMDGPU_CTX              0x02
#define DRM_AMDGPU_BO_LIST          0x03
#define DRM_AMDGPU_CS               0x04
#define DRM_AMDGPU_INFO             0x05
#define DRM_AMDGPU_GEM_METADATA     0x06
#define DRM_AMDGPU_GEM_WAIT_IDLE    0x07
#define DRM_AMDGPU_GEM_VA           0x08
#define DRM_AMDGPU_WAIT_CS          0x09
#define DRM_AMDGPU_GEM_OP           0x10
#define DRM_AMDGPU_GEM_USERPTR      0x11
#define DRM_AMDGPU_WAIT_FENCES      0x12
#define DRM_AMDGPU_VM               0x13
#define DRM_AMDGPU_FENCE_TO_HANDLE  0x14
#define DRM_AMDGPU_SCHED            0x15

// Extract ioctl nr and type bytes (Linux _IOC encoding)
// _IOC(dir, type, nr, size) = (dir << 30) | (size << 16) | (type << 8) | nr
static inline uint32_t ioc_nr(unsigned long request) {
    return request & 0xFF;
}
static inline uint32_t ioc_type(unsigned long request) {
    return (request >> 8) & 0xFF;
}

// Extract AMDGPU DRM command number (nr relative to DRM_COMMAND_BASE)
static inline uint32_t drm_amdgpu_cmd(unsigned long request) {
    uint32_t nr = ioc_nr(request);
    return nr >= DRM_COMMAND_BASE ? nr - DRM_COMMAND_BASE : 0xFFFF;
}

// Check if it's a DRM ioctl (type == 'd')
static inline bool is_drm_ioctl(unsigned long request) {
    return ioc_type(request) == DRM_IOCTL_BASE;
}

// KFD ioctl base
#define AMDKFD_IOCTL_BASE 'K'
static inline bool is_kfd_ioctl(unsigned long request) {
    return ioc_type(request) == AMDKFD_IOCTL_BASE;
}
static inline uint32_t kfd_cmd_nr(unsigned long request) {
    return ioc_nr(request);
}

using namespace flatbuffers;

namespace amdgpu_proxy {

int DispatchDrmIoctl(ProxyClient& client, unsigned long request, void* arg, int32_t virtual_fd) {
    if (!is_drm_ioctl(request)) return -ENOTTY;

    uint32_t nr = ioc_nr(request);
    FlatBufferBuilder fbb(512);
    std::vector<uint8_t> resp_buf;

    // Handle base DRM ioctls (nr < DRM_COMMAND_BASE)
    if (nr == 0x00) { // DRM_IOCTL_VERSION
        struct drm_version_layout {
            int version_major;
            int version_minor;
            int version_patchlevel;
            size_t name_len;
            char* name;
            size_t date_len;
            char* date;
            size_t desc_len;
            char* desc;
        };
        auto* ver = static_cast<drm_version_layout*>(arg);

        AmdgpuProxy::DrmVersionArgs _args(virtual_fd);
        auto req = AmdgpuProxy::CreateDrmVersionRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_DrmVersionRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();

        auto resp = msg->response_as_DrmVersionResponse();
        if (resp && resp->ret()) {
            ver->version_major = resp->ret()->version_major();
            ver->version_minor = resp->ret()->version_minor();
            ver->version_patchlevel = resp->ret()->version_patchlevel();

            auto name = AmdgpuProxy::str_data(resp->ret()->name());
            auto date = AmdgpuProxy::str_data(resp->ret()->date());
            auto desc = AmdgpuProxy::str_data(resp->ret()->desc());

            size_t name_actual = name ? std::strlen(name) : 0;
            size_t date_actual = date ? std::strlen(date) : 0;
            size_t desc_actual = desc ? std::strlen(desc) : 0;

            // Copy data if buffer provided; always report actual lengths
            // (libdrm calls VERSION twice: first to get lengths, then to get data)
            if (ver->name && name) {
                size_t len = std::min(ver->name_len, name_actual);
                std::memcpy(ver->name, name, len);
            }
            ver->name_len = name_actual;

            if (ver->date && date) {
                size_t len = std::min(ver->date_len, date_actual);
                std::memcpy(ver->date, date, len);
            }
            ver->date_len = date_actual;

            if (ver->desc && desc) {
                size_t len = std::min(ver->desc_len, desc_actual);
                std::memcpy(ver->desc, desc, len);
            }
            ver->desc_len = desc_actual;
        }
        return 0;
    }

    // AMDGPU-specific ioctls (nr >= DRM_COMMAND_BASE)
    uint32_t cmd = drm_amdgpu_cmd(request);

    switch (cmd) {
    case DRM_AMDGPU_INFO: {
        // arg points to struct drm_amdgpu_info
        struct drm_amdgpu_info_layout {
            uint64_t return_pointer;
            uint32_t return_size;
            uint32_t query;
            union { uint64_t raw[3]; } u;
        };
        auto* info = static_cast<drm_amdgpu_info_layout*>(arg);
        uint32_t sub1 = static_cast<uint32_t>(info->u.raw[0] & 0xFFFFFFFF);
        uint32_t sub2 = static_cast<uint32_t>((info->u.raw[0] >> 32) & 0xFFFFFFFF);

        AmdgpuProxy::InfoArgs _args(info->query, info->return_size, sub1, sub2, 0, static_cast<uint32_t>(virtual_fd));
        auto req = AmdgpuProxy::CreateInfoRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_InfoRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();

        auto resp = msg->response_as_InfoResponse();
        if (resp && resp->ret()->raw_data() && info->return_pointer) {
            size_t copy_size = std::min(
                static_cast<size_t>(resp->ret()->raw_data()->size()),
                static_cast<size_t>(info->return_size));
            std::memcpy(reinterpret_cast<void*>(info->return_pointer),
                        resp->ret()->raw_data()->data(), copy_size);
        }
        return 0;
    }

    case DRM_AMDGPU_GEM_CREATE: {
        struct gem_create_layout {
            uint64_t bo_size;
            uint64_t alignment;
            uint64_t domains;
            uint64_t domain_flags;
            uint32_t handle;
            uint32_t _pad;
        };
        auto* gc = static_cast<gem_create_layout*>(arg);

        AmdgpuProxy::GemCreateArgs _args(gc->bo_size, gc->alignment, gc->domains, gc->domain_flags);
        auto req = AmdgpuProxy::CreateGemCreateRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_GemCreateRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();

        auto resp = msg->response_as_GemCreateResponse();
        if (resp) gc->handle = resp->ret()->handle();
        return 0;
    }

    case DRM_AMDGPU_GEM_MMAP: {
        struct gem_mmap_layout {
            uint32_t handle;
            uint32_t _pad;
            uint64_t addr_ptr;
        };
        auto* gm = static_cast<gem_mmap_layout*>(arg);

        AmdgpuProxy::GemMmapArgs _args(gm->handle);
        auto req = AmdgpuProxy::CreateGemMmapRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_GemMmapRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();

        auto resp = msg->response_as_GemMmapResponse();
        if (resp) gm->addr_ptr = resp->ret()->addr_ptr();
        return 0;
    }

    case DRM_AMDGPU_CTX: {
        struct ctx_layout {
            uint32_t op;
            uint32_t flags;
            uint32_t ctx_id;
            int32_t priority;
            // output union
            uint32_t out_ctx_id;
            uint32_t out_pad;
            uint64_t out_flags;
            uint32_t out_hangs;
            uint32_t out_reset_status;
        };
        auto* c = static_cast<ctx_layout*>(arg);

        AmdgpuProxy::CtxArgs _args(static_cast<AmdgpuProxy::CtxOp>(c->op),
            c->flags, c->ctx_id, c->priority);
        auto req = AmdgpuProxy::CreateCtxRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_CtxRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();

        auto resp = msg->response_as_CtxResponse();
        if (resp) {
            c->out_ctx_id = resp->ret()->ctx_id();
            c->out_flags = resp->ret()->flags();
            c->out_hangs = resp->ret()->hangs();
            c->out_reset_status = resp->ret()->reset_status();
        }
        return 0;
    }

    case DRM_AMDGPU_GEM_VA: {
        struct gem_va_layout {
            uint32_t handle;
            uint32_t _pad;
            uint32_t operation;
            uint32_t flags;
            uint64_t va_address;
            uint64_t offset_in_bo;
            uint64_t map_size;
        };
        auto* va = static_cast<gem_va_layout*>(arg);

        auto req = AmdgpuProxy::CreateGemVaRequest(fbb, nullptr, AmdgpuProxy::CreateGemVaArgs(fbb,
            va->handle,
            static_cast<AmdgpuProxy::VaOp>(va->operation),
            va->flags, va->va_address, va->offset_in_bo, va->map_size));
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_GemVaRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();
        return 0;
    }

    default:
        // Unhandled DRM ioctl — return success (stub)
        return 0;
    }
}

int DispatchKfdIoctl(ProxyClient& client, unsigned long request, void* arg) {
    if (!is_kfd_ioctl(request)) return -ENOTTY;

    uint32_t cmd = kfd_cmd_nr(request);
    FlatBufferBuilder fbb(512);
    std::vector<uint8_t> resp_buf;

    switch (cmd) {
    case 0x01: { // GET_VERSION
        auto req = AmdgpuProxy::CreateKfdGetVersionRequest(fbb, nullptr, AmdgpuProxy::CreateKfdGetVersionArgs(fbb));
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdGetVersionRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();

        auto resp = msg->response_as_KfdGetVersionResponse();
        if (resp && arg) {
            struct { uint32_t major; uint32_t minor; }* v =
                static_cast<decltype(v)>(arg);
            v->major = resp->ret()->major_version();
            v->minor = resp->ret()->minor_version();
        }
        return 0;
    }

    case 0x02: { // CREATE_QUEUE
        struct kfd_create_queue_layout {
            uint64_t ring_base_address;
            uint64_t write_pointer_address;
            uint64_t read_pointer_address;
            uint64_t doorbell_offset; // out
            uint32_t ring_size;
            uint32_t gpu_id;
            uint32_t queue_type;
            uint32_t queue_percentage;
            uint32_t queue_priority;
            uint32_t queue_id; // out
            uint64_t eop_buffer_address;
            uint64_t eop_buffer_size;
            uint64_t ctx_save_restore_address;
            uint32_t ctx_save_restore_size;
            uint32_t ctl_stack_size;
            uint32_t sdma_engine_id;
            uint32_t _pad;
        };
        auto* q = static_cast<kfd_create_queue_layout*>(arg);

        AmdgpuProxy::KfdCreateQueueArgs _args(
            q->ring_base_address, q->write_pointer_address, q->read_pointer_address,
            q->ring_size, q->gpu_id,
            static_cast<AmdgpuProxy::KfdQueueType>(q->queue_type),
            q->queue_percentage, q->queue_priority,
            q->eop_buffer_address, q->eop_buffer_size,
            q->ctx_save_restore_address, q->ctx_save_restore_size,
            q->ctl_stack_size, q->sdma_engine_id);
        auto req = AmdgpuProxy::CreateKfdCreateQueueRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdCreateQueueRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        fprintf(stderr, "[shim] CREATE_QUEUE: resp_buf size=%zu\n", resp_buf.size());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) {
            fprintf(stderr, "[shim] CREATE_QUEUE FAILED: errno=%d\n", msg->error()->code());
            return -msg->error()->code();
        }

        auto resp = msg->response_as_KfdCreateQueueResponse();
        if (resp) {
            q->doorbell_offset = resp->ret()->doorbell_offset();
            q->queue_id = resp->ret()->queue_id();
            fprintf(stderr, "[shim] CREATE_QUEUE OK: queue_id=%u doorbell_offset=0x%llx\n",
                    q->queue_id, (unsigned long long)q->doorbell_offset);
        }
        return 0;
    }

    case 0x03: { // DESTROY_QUEUE
        struct { uint32_t queue_id; uint32_t _pad; }* dq =
            static_cast<decltype(dq)>(arg);

        AmdgpuProxy::KfdDestroyQueueArgs _args(dq->queue_id);
        auto req = AmdgpuProxy::CreateKfdDestroyQueueRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdDestroyQueueRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();
        return 0;
    }

    case 0x04: { // SET_MEMORY_POLICY
        struct {
            uint64_t alternate_aperture_base;
            uint64_t alternate_aperture_size;
            uint32_t gpu_id;
            uint32_t default_policy;
            uint32_t alternate_policy;
            uint32_t _pad;
        }* mp = static_cast<decltype(mp)>(arg);

        AmdgpuProxy::KfdSetMemoryPolicyArgs _args(
            mp->alternate_aperture_base, mp->alternate_aperture_size,
            mp->gpu_id,
            static_cast<AmdgpuProxy::KfdCachePolicy>(mp->default_policy),
            static_cast<AmdgpuProxy::KfdCachePolicy>(mp->alternate_policy), 0);
        auto req = AmdgpuProxy::CreateKfdSetMemoryPolicyRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdSetMemoryPolicyRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();
        return 0;
    }

    case 0x05: { // GET_CLOCK_COUNTERS
        struct {
            uint64_t gpu_clock_counter;
            uint64_t cpu_clock_counter;
            uint64_t system_clock_counter;
            uint64_t system_clock_freq;
            uint32_t gpu_id;
            uint32_t _pad;
        }* cc = static_cast<decltype(cc)>(arg);

        AmdgpuProxy::KfdGetClockCountersArgs _args(cc->gpu_id);
        auto req = AmdgpuProxy::CreateKfdGetClockCountersRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdGetClockCountersRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();

        auto resp = msg->response_as_KfdGetClockCountersResponse();
        if (resp) {
            cc->gpu_clock_counter = resp->ret()->gpu_clock_counter();
            cc->cpu_clock_counter = resp->ret()->cpu_clock_counter();
            cc->system_clock_counter = resp->ret()->system_clock_counter();
            cc->system_clock_freq = resp->ret()->system_clock_freq();
        }
        return 0;
    }

    case 0x08: { // CREATE_EVENT
        struct {
            uint64_t event_page_offset; // out
            uint32_t event_trigger_data; // out
            uint32_t event_type;
            uint32_t auto_reset;
            uint32_t node_id;
            uint32_t event_id; // out
            uint32_t event_slot_index; // out
        }* ev = static_cast<decltype(ev)>(arg);

        AmdgpuProxy::KfdCreateEventArgs _args(
            static_cast<AmdgpuProxy::KfdEventType>(ev->event_type),
            ev->auto_reset, ev->node_id);
        auto req = AmdgpuProxy::CreateKfdCreateEventRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdCreateEventRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();

        auto resp = msg->response_as_KfdCreateEventResponse();
        if (resp) {
            ev->event_page_offset = resp->ret()->event_page_offset();
            ev->event_trigger_data = resp->ret()->event_trigger_data();
            ev->event_id = resp->ret()->event_id();
            ev->event_slot_index = resp->ret()->event_slot_index();
        }
        return 0;
    }

    case 0x09: { // DESTROY_EVENT
        struct { uint32_t event_id; uint32_t pad; }* de =
            static_cast<decltype(de)>(arg);

        AmdgpuProxy::KfdDestroyEventArgs _args(de->event_id);
        auto req = AmdgpuProxy::CreateKfdDestroyEventRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdDestroyEventRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();
        return 0;
    }

    case 0x0a: { // SET_EVENT
        struct { uint32_t event_id; uint32_t pad; }* se =
            static_cast<decltype(se)>(arg);

        AmdgpuProxy::KfdSetEventArgs _args(se->event_id);
        auto req = AmdgpuProxy::CreateKfdSetEventRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdSetEventRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();
        return 0;
    }

    case 0x0b: { // RESET_EVENT
        struct { uint32_t event_id; uint32_t pad; }* re =
            static_cast<decltype(re)>(arg);

        AmdgpuProxy::KfdResetEventArgs _args(re->event_id);
        auto req = AmdgpuProxy::CreateKfdResetEventRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdResetEventRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();
        return 0;
    }

    case 0x0c: { // WAIT_EVENTS
        // WAIT_EVENTS can block for a long time. Using the shared client would
        // deadlock all other threads using the same fd. Use a dedicated temporary
        // connection instead.
        struct {
            uint64_t events_ptr;
            uint32_t num_events;
            uint32_t wait_for_all;
            uint32_t timeout;
            uint32_t wait_result;
        }* we = static_cast<decltype(we)>(arg);

        struct kfd_event_data_layout {
            uint8_t  union_data[32];
            uint64_t kfd_event_data_ext;
            uint32_t event_id;
            uint32_t pad;
        };
        auto* events = reinterpret_cast<kfd_event_data_layout*>(we->events_ptr);

        // Create a FRESH FlatBufferBuilder (don't use the shared fbb)
        FlatBufferBuilder wait_fbb(512);

        std::vector<AmdgpuProxy::KfdEventData> ev_vec;
        for (uint32_t i = 0; i < we->num_events; i++) {
            uint64_t last_age = 0;
            std::memcpy(&last_age, events[i].union_data, sizeof(uint64_t));
            ev_vec.emplace_back(events[i].event_id, last_age);
        }
        auto ev_offset = wait_fbb.CreateVectorOfStructs(ev_vec);

        auto req = AmdgpuProxy::CreateKfdWaitEventsRequest(wait_fbb, nullptr,
            AmdgpuProxy::CreateKfdWaitEventsArgs(wait_fbb, ev_offset,
                we->wait_for_all != 0, we->timeout));
        auto rpc = AmdgpuProxy::CreateRpcMessage(wait_fbb,
            AmdgpuProxy::RequestPayload_KfdWaitEventsRequest, req.Union());
        wait_fbb.Finish(rpc);

        // Use a dedicated connection for WAIT_EVENTS to avoid blocking the shared one
        ProxyClient wait_client;
        if (!wait_client.Connect()) return -EIO;

        auto wait_resp = wait_client.SendRequest(wait_fbb.GetBufferPointer(), wait_fbb.GetSize());
        if (wait_resp.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(wait_resp.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();

        auto resp = msg->response_as_KfdWaitEventsResponse();
        if (resp && resp->ret()) {
            we->wait_result = resp->ret()->wait_result();
            if (resp->ret()->events() && resp->ret()->events()->size() > 0) {
                auto raw = resp->ret()->events();
                size_t copy_sz = std::min(static_cast<size_t>(raw->size()),
                    static_cast<size_t>(we->num_events * sizeof(kfd_event_data_layout)));
                std::memcpy(events, raw->data(), copy_sz);
            }
        }
        return 0;
    }

    case 0x14: { // GET_PROCESS_APERTURES_NEW
        AmdgpuProxy::KfdGetProcessAperturesArgs _args(32);
        auto req = AmdgpuProxy::CreateKfdGetProcessAperturesRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdGetProcessAperturesRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();

        // Write aperture data back to the caller's struct
        auto resp = msg->response_as_KfdGetProcessAperturesResponse();
        if (resp && resp->ret()->apertures() && arg) {
            struct kfd_process_device_apertures_layout {
                uint64_t lds_base, lds_limit;
                uint64_t scratch_base, scratch_limit;
                uint64_t gpuvm_base, gpuvm_limit;
                uint32_t gpu_id;
                uint32_t _pad;
            };
            // The ioctl arg struct:
            // { uint64_t kfd_process_device_apertures_ptr; uint32_t num_of_nodes; uint32_t _pad; }
            struct {
                uint64_t ptr;
                uint32_t num_of_nodes;
                uint32_t _pad;
            }* ap = static_cast<decltype(ap)>(arg);

            auto* aps = reinterpret_cast<kfd_process_device_apertures_layout*>(ap->ptr);
            uint32_t n = std::min(ap->num_of_nodes,
                static_cast<uint32_t>(resp->ret()->apertures()->size()));
            for (uint32_t i = 0; i < n; i++) {
                auto& a = *resp->ret()->apertures()->Get(i);
                aps[i].lds_base = a.lds_base();
                aps[i].lds_limit = a.lds_limit();
                aps[i].scratch_base = a.scratch_base();
                aps[i].scratch_limit = a.scratch_limit();
                aps[i].gpuvm_base = a.gpuvm_base();
                aps[i].gpuvm_limit = a.gpuvm_limit();
                aps[i].gpu_id = a.gpu_id();
                aps[i]._pad = 0;
            }
            ap->num_of_nodes = n;
        }
        return 0;
    }

    case 0x15: { // ACQUIRE_VM
        struct { uint32_t drm_fd; uint32_t gpu_id; }* av =
            static_cast<decltype(av)>(arg);

        AmdgpuProxy::KfdAcquireVmArgs _args(av->drm_fd, av->gpu_id);
        auto req = AmdgpuProxy::CreateKfdAcquireVmRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdAcquireVmRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();
        return 0;
    }

    case 0x16: { // ALLOC_MEMORY_OF_GPU
        struct {
            uint64_t va_addr;
            uint64_t size;
            uint64_t handle; // out
            uint64_t mmap_offset; // out
            uint32_t gpu_id;
            uint32_t flags;
        }* am = static_cast<decltype(am)>(arg);

        fprintf(stderr, "[shim] ALLOC_MEMORY: va=0x%llx size=0x%llx gpu=%u flags=0x%x\n",
                (unsigned long long)am->va_addr, (unsigned long long)am->size,
                am->gpu_id, am->flags);

        uint64_t orig_va_addr = am->va_addr;

        AmdgpuProxy::KfdAllocMemoryArgs _args(
            am->va_addr, am->size, am->gpu_id, am->flags, am->mmap_offset);
        auto req = AmdgpuProxy::CreateKfdAllocMemoryRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdAllocMemoryRequest, req.Union());
        fbb.Finish(rpc);

        int received_fd = -1;
        resp_buf = client.SendRequestReceiveFd(fbb.GetBufferPointer(), fbb.GetSize(), &received_fd);
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) {
            if (received_fd >= 0) syscall(SYS_close, received_fd);
            return -msg->error()->code();
        }

        auto resp = msg->response_as_KfdAllocMemoryResponse();
        if (resp) {
            am->handle = resp->ret()->handle();
            am->mmap_offset = resp->ret()->mmap_offset();
            am->va_addr = resp->ret()->va_addr();
        }

        // If we received a memfd (USERPTR case), map it over the client's
        // anonymous pages so both client and server share the same physical pages.
        if (received_fd >= 0 && orig_va_addr != 0) {
            typedef void* (*mmap_fn)(void*, size_t, int, int, int, off_t);
            static mmap_fn sys_mmap = reinterpret_cast<mmap_fn>(
                dlsym(RTLD_NEXT, "mmap"));
            void* ptr = sys_mmap(reinterpret_cast<void*>(orig_va_addr), am->size,
                                 PROT_READ | PROT_WRITE,
                                 MAP_SHARED | MAP_FIXED, received_fd, 0);
            if (ptr == MAP_FAILED) {
                // Keep anonymous pages rather than failing the whole allocation
                syscall(SYS_close, received_fd);
            }
            // Keep fd open to maintain the mapping
        }

        return 0;
    }

    case 0x17: { // FREE_MEMORY_OF_GPU
        struct { uint64_t handle; }* fm = static_cast<decltype(fm)>(arg);

        AmdgpuProxy::KfdFreeMemoryArgs _args(fm->handle);
        auto req = AmdgpuProxy::CreateKfdFreeMemoryRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdFreeMemoryRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();
        return 0;
    }

    case 0x18: { // MAP_MEMORY_TO_GPU
        struct {
            uint64_t handle;
            uint64_t device_ids_array_ptr;
            uint32_t n_devices;
            uint32_t n_success; // out
        }* mm = static_cast<decltype(mm)>(arg);

        auto* devs = reinterpret_cast<uint32_t*>(mm->device_ids_array_ptr);
        std::vector<uint32_t> dev_vec(devs, devs + mm->n_devices);
        auto dev_off = fbb.CreateVector(dev_vec);

        auto args_off = AmdgpuProxy::CreateKfdMapMemoryToGpuArgs(fbb, mm->handle, dev_off);
        auto req = AmdgpuProxy::CreateKfdMapMemoryToGpuRequest(fbb, nullptr, args_off);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdMapMemoryToGpuRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();

        auto resp = msg->response_as_KfdMapMemoryToGpuResponse();
        if (resp) mm->n_success = resp->ret()->n_success();
        return 0;
    }

    case 0x19: { // UNMAP_MEMORY_FROM_GPU
        struct {
            uint64_t handle;
            uint64_t device_ids_array_ptr;
            uint32_t n_devices;
            uint32_t n_success; // out
        }* um = static_cast<decltype(um)>(arg);

        auto* devs = reinterpret_cast<uint32_t*>(um->device_ids_array_ptr);
        std::vector<uint32_t> dev_vec(devs, devs + um->n_devices);
        auto dev_off = fbb.CreateVector(dev_vec);

        auto args_off = AmdgpuProxy::CreateKfdUnmapMemoryFromGpuArgs(fbb, um->handle, dev_off);
        auto req = AmdgpuProxy::CreateKfdUnmapMemoryFromGpuRequest(fbb, nullptr, args_off);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdUnmapMemoryFromGpuRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();

        auto resp = msg->response_as_KfdUnmapMemoryFromGpuResponse();
        if (resp) um->n_success = resp->ret()->n_success();
        return 0;
    }

    case 0x20: { // SVM
        struct kfd_svm_layout {
            uint64_t start_addr;
            uint64_t size;
            uint32_t op;
            uint32_t nattr;
            struct { uint32_t type; uint32_t value; } attrs[];
        };
        auto* sv = static_cast<kfd_svm_layout*>(arg);

        fprintf(stderr, "[shim] SVM: op=%u start=0x%llx size=0x%llx nattr=%u\n",
                sv->op, (unsigned long long)sv->start_addr,
                (unsigned long long)sv->size, sv->nattr);

        // Build attributes vector for FlatBuffer
        std::vector<AmdgpuProxy::KfdSvmAttribute> fb_attrs(sv->nattr);
        for (uint32_t i = 0; i < sv->nattr; i++) {
            fb_attrs[i] = AmdgpuProxy::KfdSvmAttribute(sv->attrs[i].type, sv->attrs[i].value);
        }

        auto attrs_vec = fbb.CreateVectorOfStructs(fb_attrs.data(), fb_attrs.size());
        auto svm_args = AmdgpuProxy::CreateKfdSvmArgs(fbb,
            sv->start_addr, sv->size,
            static_cast<AmdgpuProxy::KfdSvmOp>(sv->op), attrs_vec);
        auto req = AmdgpuProxy::CreateKfdSvmRequest(fbb, nullptr, svm_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdSvmRequest, req.Union());
        fbb.Finish(rpc);

        int received_fd = -1;
        resp_buf = client.SendRequestReceiveFd(fbb.GetBufferPointer(), fbb.GetSize(), &received_fd);
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) {
            if (received_fd >= 0) syscall(SYS_close, received_fd);
            fprintf(stderr, "[shim] SVM FAILED: errno=%d\n", msg->error()->code());
            return -msg->error()->code();
        }

        // Map memfd at client's SVM address so pages are shared with server
        if (received_fd >= 0 && sv->start_addr != 0 && sv->op == 0 /* SET_ATTR */) {
            typedef void* (*mmap_fn)(void*, size_t, int, int, int, off_t);
            static mmap_fn sys_mmap = reinterpret_cast<mmap_fn>(
                dlsym(RTLD_NEXT, "mmap"));
            void* ptr = sys_mmap(reinterpret_cast<void*>(sv->start_addr), sv->size,
                                 PROT_READ | PROT_WRITE,
                                 MAP_SHARED | MAP_FIXED, received_fd, 0);
            if (ptr == MAP_FAILED) {
                fprintf(stderr, "[shim] SVM mmap at client addr FAILED: %s\n", strerror(errno));
                syscall(SYS_close, received_fd);
            } else {
                fprintf(stderr, "[shim] SVM mapped memfd at 0x%llx size=0x%llx\n",
                        (unsigned long long)sv->start_addr, (unsigned long long)sv->size);
            }
        }

        return 0;
    }

    case 0x21: { // SET_XNACK_MODE
        struct { int32_t xnack_enabled; }* xn = static_cast<decltype(xn)>(arg);

        AmdgpuProxy::KfdSetXnackModeArgs _args(xn->xnack_enabled);
        auto req = AmdgpuProxy::CreateKfdSetXnackModeRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdSetXnackModeRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();

        auto resp = msg->response_as_KfdSetXnackModeResponse();
        if (resp) xn->xnack_enabled = resp->ret()->xnack_enabled();
        return 0;
    }

    case 0x23: { // AVAILABLE_MEMORY
        struct { uint64_t available; uint32_t gpu_id; uint32_t pad; }* am =
            static_cast<decltype(am)>(arg);
        AmdgpuProxy::KfdAvailableMemoryArgs _args(am->gpu_id);
        auto req = AmdgpuProxy::CreateKfdAvailableMemoryRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdAvailableMemoryRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();

        auto resp = msg->response_as_KfdAvailableMemoryResponse();
        if (resp && arg) {
            struct { uint64_t available; uint32_t gpu_id; uint32_t pad; }* am =
                static_cast<decltype(am)>(arg);
            am->available = resp->ret()->available();
        }
        return 0;
    }

    case 0x25: { // RUNTIME_ENABLE
        struct {
            uint64_t r_debug;
            uint32_t mode_mask;
            uint32_t capabilities_mask;
        }* re = static_cast<decltype(re)>(arg);

        fprintf(stderr, "[shim] RUNTIME_ENABLE: mode=0x%x caps=0x%x r_debug=0x%llx\n",
                re->mode_mask, re->capabilities_mask, (unsigned long long)re->r_debug);

        // Pack mode_mask + capabilities_mask into flags (r_debug is not valid in server)
        uint64_t flags = static_cast<uint64_t>(re->mode_mask) |
                         (static_cast<uint64_t>(re->capabilities_mask) << 32);
        AmdgpuProxy::KfdRuntimeEnableArgs _args(flags);
        auto req = AmdgpuProxy::CreateKfdRuntimeEnableRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdRuntimeEnableRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) {
            fprintf(stderr, "[shim] RUNTIME_ENABLE FAILED: errno=%d\n", msg->error()->code());
            return -msg->error()->code();
        }

        fprintf(stderr, "[shim] RUNTIME_ENABLE OK\n");
        return 0;
    }

    case 0x11: { // SET_SCRATCH_BACKING_VA
        struct {
            uint64_t va_addr;
            uint32_t gpu_id;
            uint32_t _pad;
        }* sb = static_cast<decltype(sb)>(arg);

        AmdgpuProxy::KfdSetScratchBackingVaArgs _args(sb->va_addr, sb->gpu_id, 0);
        auto req = AmdgpuProxy::CreateKfdSetScratchBackingVaRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdSetScratchBackingVaRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();
        return 0;
    }

    case 0x13: { // SET_TRAP_HANDLER
        struct {
            uint64_t tba_addr;
            uint64_t tma_addr;
            uint32_t gpu_id;
            uint32_t _pad;
        }* th = static_cast<decltype(th)>(arg);

        AmdgpuProxy::KfdSetTrapHandlerArgs _args(th->tba_addr, th->tma_addr, th->gpu_id, 0);
        auto req = AmdgpuProxy::CreateKfdSetTrapHandlerRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_KfdSetTrapHandlerRequest, req.Union());
        fbb.Finish(rpc);

        resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        if (resp_buf.empty()) return -EIO;

        auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
        if (!msg) return -EIO;
        if (msg->error()) return -msg->error()->code();
        return 0;
    }

    default:
        // Unhandled KFD ioctl — return success (stub)
        fprintf(stderr, "[shim] UNHANDLED KFD ioctl cmd=0x%x\n", cmd);
        return 0;
    }
}

int DispatchSysfsRead(ProxyClient& client, const char* path,
                      void* buffer, uint32_t max_size) {
    FlatBufferBuilder fbb(256);
    AmdgpuProxy::SysfsReadArgs _args(AmdgpuProxy::make_str<AmdgpuProxy::Str255>(path), max_size);
    auto req = AmdgpuProxy::CreateSysfsReadRequest(fbb, nullptr, &_args);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_SysfsReadRequest, req.Union());
    fbb.Finish(rpc);

    auto resp_buf = client.SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
    if (resp_buf.empty()) return -EIO;

    auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
    if (!msg) return -EIO;
    if (msg->error()) return -msg->error()->code();

    auto resp = msg->response_as_SysfsReadResponse();
    if (!resp || !resp->ret()->data()) return 0;

    size_t copy_size = std::min(static_cast<size_t>(resp->ret()->data()->size()),
                                static_cast<size_t>(max_size));
    std::memcpy(buffer, resp->ret()->data()->data(), copy_size);
    return static_cast<int>(copy_size);
}

} // namespace amdgpu_proxy
