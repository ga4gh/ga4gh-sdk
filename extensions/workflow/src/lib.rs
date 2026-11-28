//! Workflow provider for the GA4GH C ABI v1.
mod workflow;

use ga4gh_cli_protocol::{encode, Output, Reply, CAPABILITY, MAX_MESSAGE};
use ga4gh_extension_abi as abi;
use ga4gh_sdk::workflow::PreparedInvocation;
use std::{
    ffi::c_void,
    panic::{catch_unwind, AssertUnwindSafe},
    ptr,
};

type Pending = Option<PreparedInvocation>;

unsafe fn bytes<'a>(input: abi::Bytes) -> Option<&'a [u8]> {
    if input.len == 0 {
        Some(&[])
    } else if input.len > MAX_MESSAGE || input.data.is_null() {
        None
    } else {
        Some(unsafe { std::slice::from_raw_parts(input.data, input.len) })
    }
}

unsafe extern "C" fn create(
    _: abi::Bytes,
    instance: *mut *mut c_void,
    _: *mut abi::OwnedBytes,
) -> i32 {
    unsafe { instance.write(Box::into_raw(Box::new(Pending::None)).cast()) };
    abi::OK
}

unsafe extern "C" fn invoke(
    instance: *mut c_void,
    capability: abi::Bytes,
    revision: u32,
    operation: abi::Bytes,
    input: abi::Bytes,
    output: *mut abi::OwnedBytes,
) -> i32 {
    let reply = catch_unwind(AssertUnwindSafe(|| -> Option<Vec<u8>> {
        if revision != 1 || unsafe { bytes(capability)? } != CAPABILITY.as_bytes() {
            return None;
        }
        let pending = unsafe { instance.cast::<Pending>().as_mut()? };
        let input = unsafe { bytes(input)? };
        match unsafe { bytes(operation)? } {
            b"execute" => {
                let invocation = serde_json::from_slice(input).ok()?;
                let reply = workflow::execute(invocation, pending)
                    .unwrap_or_else(|error| Reply::Complete(Output::failure(error)));
                encode(&reply)
                    .or_else(|error| encode(&Reply::Complete(Output::failure(error))))
                    .ok()
            }
            b"wes_outcome" => encode(
                &workflow::finish(pending.take()?, serde_json::from_slice(input).ok()?).ok()?,
            )
            .ok(),
            _ => None,
        }
    }));
    let Ok(Some(reply)) = reply else { return 1 };
    let buffer = reply.into_boxed_slice();
    unsafe {
        output.write(abi::OwnedBytes {
            len: buffer.len(),
            data: Box::into_raw(buffer).cast(),
        })
    };
    abi::OK
}

unsafe extern "C" fn destroy(instance: *mut c_void) {
    unsafe { drop(Box::from_raw(instance.cast::<Pending>())) };
}
unsafe extern "C" fn free(_: *mut c_void, buffer: abi::OwnedBytes) {
    if !buffer.data.is_null() {
        unsafe {
            drop(Box::from_raw(ptr::slice_from_raw_parts_mut(
                buffer.data,
                buffer.len,
            )))
        };
    }
}

static DESCRIPTOR: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/descriptor.json"));
static API: abi::ApiV1 = abi::ApiV1 {
    header: abi::Header {
        abi_version: abi::ABI_VERSION,
        struct_size: size_of::<abi::ApiV1>() as u32,
    },
    descriptor: abi::Bytes {
        data: DESCRIPTOR.as_ptr(),
        len: DESCRIPTOR.len(),
    },
    create_instance: Some(create),
    invoke: Some(invoke),
    destroy_instance: Some(destroy),
    free_buffer: Some(free),
};

#[no_mangle]
pub extern "C" fn ga4gh_extension_get_api_v1() -> *const abi::ApiV1 {
    &API
}
