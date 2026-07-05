# Jackstay native ABI is descriptor, lease, and release based

The macOS native path proved the handle-first design in ADR-0004, but its public
C ABI is still shaped like the first backend: `IOSurfaceRef[]` plus one
`MTLSharedEventHandle`, read through `ft_native_read_latest`. ADR-0008 names the
Linux backend as the freeze gate precisely because Linux breaks that shape:
dmabuf handles can be multi-plane, modifiers matter, sync is fd/timeline based,
and dmabuf has no equivalent to `IOSurfaceGetUseCount`.

Before implementing Linux, re-cut the native ABI around the cross-platform
truths the backends share:

- surfaces and sync objects are **typed descriptors**, not raw platform-shaped
  fields;
- frame acquisition creates an explicit **lease**;
- consumers explicitly **release** leases, either now or by naming a registered
  release timeline value;
- waiting for frame availability is separate from acquiring a lease;
- pool reconfiguration is part of the native contract, not an out-of-band
  reconnect assumption.

This is a clean break. Jackstay is pre-release, so the old macOS-only native C
ABI is replaced rather than wrapped.

## Native descriptors

Every native C ABI struct that crosses a function boundary carries
`struct_size`; callers set it on input, and the library fills it on output. This
keeps tail extension possible while still allowing strict validation and
`_Static_assert` layout checks.

Surface handles are discriminated:

```c
#define FT_NATIVE_HANDLE_IOSURFACE 1
#define FT_NATIVE_HANDLE_DMABUF 2
#define FT_NATIVE_HANDLE_D3D12_RESOURCE 3
#define FT_NATIVE_MAX_PLANES 4

typedef struct ft_native_plane {
  int32_t fd;
  uint32_t offset;
  uint32_t stride;
} ft_native_plane;

typedef struct ft_native_surface {
  uint32_t struct_size;
  uint32_t handle_kind;
  uint32_t plane_count;
  uint32_t width;
  uint32_t height;
  uint32_t pixel_format;
  uint64_t modifier;
  void *object;
  ft_native_plane planes[FT_NATIVE_MAX_PLANES];
} ft_native_surface;
```

`object` is for object-shaped handles such as `IOSurfaceRef` or a future D3D
resource wrapper. dmabuf uses `planes[].fd`; `object` is null. Repeating the same
fd across planes is allowed and means the planes share one borrowed dmabuf.
`modifier` is per surface, not per plane. The fixed four-plane cap follows the
DRM/dmabuf model: RGBA/BGRA is one memory plane, common YUV formats use two or
three, and four is the practical descriptor ceiling without adding nested
allocation lifetime rules to the ABI.

Sync handles are also discriminated:

```c
#define FT_NATIVE_SYNC_NONE 0
#define FT_NATIVE_SYNC_MTL_SHARED_EVENT 1
#define FT_NATIVE_SYNC_DRM_SYNCOBJ_TIMELINE 2
#define FT_NATIVE_SYNC_D3D12_FENCE 3

typedef struct ft_native_sync {
  uint32_t struct_size;
  uint32_t sync_kind;
  uint64_t sync_id;
  union {
    void *object;
    int32_t fd;
  } handle;
} ft_native_sync;
```

Grant handles are borrowed. The `ft_native_attach` owns the platform handles and
fds exposed by grant, pool, surface, and producer-sync descriptors; they are
valid until `ft_native_attach_destroy` or until a later reconfiguration contract
retires them after outstanding leases drain. C consumers do not close grant fds.
If a consumer needs a longer-lived imported object, it imports/retains through
the platform API before destroying the attach.

## Attach descriptors and grants

The stable connect entry point is descriptor based, not platform-function based:

```c
#define FT_NATIVE_ATTACH_TRANSPORT_MACOS_XPC 1
#define FT_NATIVE_ATTACH_TRANSPORT_UNIX_SOCKET 2

typedef struct ft_native_attach_descriptor {
  uint32_t struct_size;
  uint32_t transport_kind;
  uint64_t requested_consumer_id;
  const char *endpoint;
  const char *bearer_token;
  uint32_t flags;
} ft_native_attach_descriptor;
```

`requested_consumer_id == 0` means "assign one"; the grant returns the actual
identity. For macOS, `endpoint` is the Mach service name. For Linux, it is the
Unix-domain socket path. A future copy-pasteable or URL-like locator can be
layered over this descriptor, but the in-memory C struct remains the immediate
connect contract.

The grant is an initial snapshot of pools plus the producer readiness timeline:

```c
typedef struct ft_native_pool {
  uint32_t struct_size;
  uint32_t surface_count;
  uint64_t pool_id;
  const ft_native_surface *surfaces;
} ft_native_pool;

typedef struct ft_native_grant {
  uint32_t struct_size;
  uint32_t pool_count;
  uint64_t consumer_id;
  uint64_t consumer_slot;
  const ft_native_pool *pools;
  ft_native_sync producer_sync;
} ft_native_grant;
```

The ABI uses an array of pools from day one even though current streams normally
grant one pool. Frames name `{pool_id, slot_id}` to avoid assuming one eternal
swapchain.

## Acquire, wait, and release

Reading latest is not enough for Linux correctness. Acquisition creates a lease:

```c
typedef struct ft_native_frame {
  uint32_t struct_size;
  uint32_t flags;
  uint64_t lease_id;
  uint64_t cursor;
  uint64_t sequence;
  uint64_t timestamp_ns;
  uint64_t pool_id;
  uint32_t slot_id;
  uint32_t width;
  uint32_t height;
  uint32_t pixel_format;
  uint64_t producer_sync_id;
  uint64_t producer_sync_value;
} ft_native_frame;

ft_status ft_native_acquire_latest(
  ft_native_attach *attach,
  uint64_t min_cursor,
  ft_native_frame *out_frame
);
```

`min_cursor` is caller-owned cursor state. `0` means acquire the newest currently
available frame. Passing the last seen cursor returns `FT_STATUS_EMPTY` unless a
newer frame exists. Each successful acquire creates a distinct lease; consumers
may hold multiple outstanding leases and release each exactly once by
`lease_id`.

Waiting is explicit and does not create a lease:

```c
#define FT_WAIT_INFINITE UINT64_MAX

ft_status ft_native_wait_frame(
  ft_native_attach *attach,
  uint64_t min_cursor,
  uint64_t timeout_ns,
  uint64_t *out_cursor
);
```

The function waits until the producer cursor advances past `min_cursor`, the
timeout expires, or the session closes. It returns the newest known cursor at
wake. Event-loop handle export is intentionally deferred; `ft_native_wait_frame`
is the portable blocking primitive. macOS may later need a kqueue-compatible
bridge, and Linux may later expose a pollable fd, but that is additive.

Release is explicit:

```c
#define FT_NATIVE_RELEASE_NOW 1
#define FT_NATIVE_RELEASE_TIMELINE_VALUE 2

typedef struct ft_native_release {
  uint32_t struct_size;
  uint32_t release_kind;
  uint64_t lease_id;
  uint64_t release_sync_id;
  uint64_t release_value;
} ft_native_release;

ft_status ft_native_release_frame(
  ft_native_attach *attach,
  const ft_native_release *release
);
```

`FT_NATIVE_RELEASE_NOW` means all CPU and GPU work touching the leased surface is
complete now. It is not a convenience fiction for "work has been queued".
Asynchronous GPU consumers register a release timeline once and then release
each frame by timeline value:

```c
ft_status ft_native_register_release_sync(
  ft_native_attach *attach,
  const ft_native_sync *sync,
  uint64_t *out_release_sync_id
);
```

No per-frame release fd transfer is part of the steady state.

## Reconfiguration

Resize, format change, and swapchain recreation are part of the native contract.
The initial grant is a snapshot, and later events announce changes:

```c
#define FT_NATIVE_EVENT_POOL_ADDED 1
#define FT_NATIVE_EVENT_POOL_REMOVED 2
#define FT_NATIVE_EVENT_STREAM_CONFIG_CHANGED 3
#define FT_NATIVE_EVENT_PRODUCER_STOPPED 4

typedef struct ft_native_event {
  uint32_t struct_size;
  uint32_t kind;
  uint64_t pool_id;
  uint64_t config_generation;
} ft_native_event;

ft_status ft_native_poll_event(
  ft_native_attach *attach,
  ft_native_event *out_event
);

ft_status ft_native_get_pool(
  ft_native_attach *attach,
  uint64_t pool_id,
  ft_native_pool *out_pool
);
```

`POOL_ADDED` means the consumer can fetch descriptors for the new pool.
`POOL_REMOVED` means no new frames should refer to the pool, but descriptors and
handles remain valid until outstanding leases and release waits have drained.
`STREAM_CONFIG_CHANGED` exposes generation changes; frame and surface
descriptors still carry the concrete dimensions and format used to import and
sample. `PRODUCER_STOPPED` is the terminal control signal.

## Reuse gating

The internal Rust seam must stop asking backends for `surface_use_count`.
`IOSurfaceGetUseCount` is a macOS implementation detail, not the cross-platform
contract. Reuse is a slot-acquisition operation:

- the neutral core supplies slots whose last publication is outside the live ring
  window;
- the backend/reuse policy chooses a reusable native slot, waits for release
  completion if the caller requested waiting, or reports that no slot is
  available;
- producer exhaustion policy still decides whether to drop, fail, or wait.

For macOS, the reuse guard can continue to use IOSurface holds/in-use state. For
Linux, reuse is based on outstanding leases plus registered consumer release
timeline completion. The public C ABI is therefore callable from C/Zig without
requiring those consumers to participate in hidden Rust-only state.

## Linux sequencing

Implement Linux in two steps:

1. Synthetic dmabuf producer plus Vulkan consumer reference. This must use real
   dmabuf buffers, real drm_syncobj timeline synchronization, the descriptor C
   ABI, explicit leases, release timelines, and Linux UDS attach over
   `SCM_RIGHTS`.
2. PipeWire/dmabuf producer. This proves the real desktop capture path and is
   required before the ADR-0008 freeze gate is complete.

The Linux attach transport uses a descriptor plus fd table: JSON/control
messages refer to fd indices, and the actual fds cross as `SCM_RIGHTS`
ancillary data. The public C grant exposes direct borrowed fd numbers after the
library receives and owns them.

Use small Linux FFI shims for DRM, Vulkan, and PipeWire integration, mirroring
the narrow macOS shim boundary, rather than growing a large graphics dependency
surface in `capture-transfer`.

## Status codes

The native ABI needs statuses beyond the original four:

```c
#define FT_STATUS_TIMEOUT 4
#define FT_STATUS_CLOSED 5
#define FT_STATUS_UNSUPPORTED 6
#define FT_STATUS_INVALID_STATE 7
```

`EMPTY` remains "no frame/event currently available". `TIMEOUT` is a wait
expiry. `CLOSED` means producer/session closed. `UNSUPPORTED` means unsupported
transport, handle, sync, or backend. `INVALID_STATE` covers bad ordering, double
release, unknown lease, and similar state-machine violations.
