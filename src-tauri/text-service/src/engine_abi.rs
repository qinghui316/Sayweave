//! ABI1: fixed C layouts, caller-owned buffers and engine-owned opaque handles.
//! No Rust collection, COM implementation or allocator ownership crosses the boundary.
use std::ffi::c_void;

pub const ENGINE_ABI: u32 = 1;
pub const CONTROL_PROTOCOL: u32 = 1;
pub const BUSY: i32 = 0x800700aa_u32 as i32;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct EngineStatusV1 {
    pub abi: u32,
    pub input_protocol: u32,
    pub generation: u64,
    pub build_id: u64,
    pub module_base: u64,
    pub module_sha256: [u8; 32],
    pub active: u32,
    pub ranges: u32,
    pub pending: u32,
    pub cleanup: u32,
    pub observers: u32,
    pub callbacks: u32,
    pub leases: u32,
}
impl EngineStatusV1 {
    pub fn idle(&self) -> bool {
        self.ranges == 0
            && self.pending == 0
            && self.cleanup == 0
            && self.observers == 0
            && self.callbacks == 0
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct EngineApiV1 {
    pub abi: u32,
    pub size: u32,
    pub handle: *mut c_void,
    pub bind_host: unsafe extern "system" fn(*mut c_void, *const HostGuardV1) -> i32,
    pub disconnect: unsafe extern "system" fn(*mut c_void) -> i32,
    pub activate: unsafe extern "system" fn(*mut c_void, *mut c_void, u32) -> i32,
    pub dispatch: unsafe extern "system" fn(*mut c_void, *const u8, usize) -> i32,
    pub poll: unsafe extern "system" fn(*mut c_void) -> i32,
    pub read: unsafe extern "system" fn(*mut c_void, *mut u8, usize, *mut usize) -> i32,
    pub status: unsafe extern "system" fn(*mut c_void, *mut EngineStatusV1) -> i32,
    pub deactivate: unsafe extern "system" fn(*mut c_void) -> i32,
    pub retire: unsafe extern "system" fn(*mut c_void) -> i32,
    pub destroy: unsafe extern "system" fn(*mut c_void) -> i32,
    pub retain: unsafe extern "system" fn(*mut c_void) -> i32,
    pub release: unsafe extern "system" fn(*mut c_void) -> i32,
}
pub type CreateEngineV1 = unsafe extern "system" fn(u64, *mut EngineApiV1, usize) -> i32;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostGuardV1 {
    pub context: *mut c_void,
    pub proxy_instance: [u8; 16],
    pub engine_generation: u64,
    pub validate: unsafe extern "system" fn(*mut c_void, u64, u32, *const u8, usize) -> i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct ProxyStatusV1 {
    pub abi: u32,
    pub ready: u32,
    pub instance: [u8; 16],
    pub generation: u64,
    pub retired: u32,
    pub engine: EngineStatusV1,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn each_native_resource_blocks_retirement() {
        for field in 0..5 {
            let mut state = EngineStatusV1::default();
            match field {
                0 => state.ranges = 1,
                1 => state.pending = 1,
                2 => state.cleanup = 1,
                3 => state.observers = 1,
                _ => state.callbacks = 1,
            }
            assert!(!state.idle());
        }
        assert!(
            EngineStatusV1 {
                leases: 1,
                ..Default::default()
            }
            .idle()
        );
    }
}
