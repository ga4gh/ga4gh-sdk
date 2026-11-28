//! C ABI v1. See `include/ga4gh_extension.h` for ownership and threading rules.
#![no_std]
use core::ffi::c_void;

/// Supported native ABI revision.
pub const ABI_VERSION: u32 = 1;
/// The single required exported symbol, including its terminating NUL.
pub const ENTRY_POINT: &[u8] = b"ga4gh_extension_get_api_v1\0";
/// Zero indicates success; all nonzero statuses indicate plugin failure.
pub const OK: i32 = 0;

/// Borrowed bytes, valid for the duration specified by the enclosing operation.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Bytes {
    /// Null is permitted only when `len` is zero.
    pub data: *const u8,
    /// Byte count.
    pub len: usize,
}
impl Bytes {
    /// Borrows a slice until the foreign call returns.
    pub fn new(bytes: &[u8]) -> Self {
        Self {
            data: bytes.as_ptr(),
            len: bytes.len(),
        }
    }
}
/// Plugin-allocated bytes; only the plugin's `free_buffer` may release them.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct OwnedBytes {
    /// Null is permitted only when `len` is zero.
    pub data: *mut u8,
    /// Byte count. Allocation metadata remains private to the plugin.
    pub len: usize,
}
impl Default for OwnedBytes {
    fn default() -> Self {
        Self {
            data: core::ptr::null_mut(),
            len: 0,
        }
    }
}
/// Fixed prefix shared by all API table revisions.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Header {
    /// Native ABI revision.
    pub abi_version: u32,
    /// Total table size in bytes, including this header.
    pub struct_size: u32,
}
/// Creates an opaque instance. Failure leaves `instance` null.
pub type CreateInstance =
    unsafe extern "C" fn(config: Bytes, instance: *mut *mut c_void, error: *mut OwnedBytes) -> i32;
/// Calls one operation; `output` holds either success bytes or private error bytes.
pub type Invoke = unsafe extern "C" fn(
    instance: *mut c_void,
    capability: Bytes,
    revision: u32,
    operation: Bytes,
    input: Bytes,
    output: *mut OwnedBytes,
) -> i32;
/// Destroys an instance after every invocation and result release finishes.
pub type DestroyInstance = unsafe extern "C" fn(instance: *mut c_void);
/// Releases a returned buffer; instance is null for creation errors.
pub type FreeBuffer = unsafe extern "C" fn(instance: *mut c_void, buffer: OwnedBytes);
/// Immutable module-owned table, valid until the module is released.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ApiV1 {
    /// Validate this prefix before accessing any other fields.
    pub header: Header,
    /// UTF-8 JSON: name, version, target, abi_version, capabilities and optional capability_metadata.
    pub descriptor: Bytes,
    /// Required non-null function pointer.
    pub create_instance: Option<CreateInstance>,
    /// Required non-null function pointer.
    pub invoke: Option<Invoke>,
    /// Required non-null function pointer.
    pub destroy_instance: Option<DestroyInstance>,
    /// Required non-null function pointer.
    pub free_buffer: Option<FreeBuffer>,
}
// SAFETY: an ABI-conforming published table and its descriptor are immutable and
// remain valid while loaded. Reading it does not access instance state.
unsafe impl Sync for ApiV1 {}
/// Export this function with the name in `ENTRY_POINT`.
pub type GetApiV1 = unsafe extern "C" fn() -> *const ApiV1;
