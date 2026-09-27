//! Host-side COM objects handed to plugins: `IHostApplication` (which also creates
//! `IMessage`/`IAttributeList` for `IConnectionPoint` messaging) and `IComponentHandler`.
//!
//! Callbacks only record what happened; [`crate::Vst3Plugin::poll`] turns the records into
//! `PluginNotification`s on the plugin main thread.

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use vst3::Steinberg::Vst::{
    IAttributeList, IAttributeListTrait, IComponentHandler, IComponentHandler2,
    IComponentHandler2Trait, IComponentHandlerTrait, IHostApplication, IHostApplicationTrait,
    IMessage, IMessageTrait, ParamID, ParamValue, String128, TChar,
};
use vst3::Steinberg::{
    FIDString, TBool, TUID, int32, int64, kInvalidArgument, kNoInterface, kResultFalse, kResultOk,
    tresult, uint32,
};
use vst3::{Class, ComWrapper, Interface};

pub(crate) const HOST_NAME: &str = "Ethereal";

/// Copy `s` into a NUL-terminated UTF-16 buffer (truncating).
pub(crate) fn write_tchar(s: &str, dst: &mut [TChar]) {
    let Some(last) = dst.len().checked_sub(1) else {
        return;
    };
    let mut n = 0;
    for (d, c) in dst[..last].iter_mut().zip(s.encode_utf16()) {
        *d = c as TChar;
        n += 1;
    }
    dst[n] = 0;
}

/// Read a NUL-terminated (or full) UTF-16 buffer.
pub(crate) fn read_tchar(src: &[TChar]) -> String {
    let len = src.iter().position(|c| *c == 0).unwrap_or(src.len());
    String::from_utf16_lossy(&src[..len]).trim().to_owned()
}

/// Read a NUL-terminated (or full) `char8` buffer.
pub(crate) fn read_char8(src: &[c_char]) -> String {
    let bytes: Vec<u8> = src
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).trim().to_owned()
}

fn tuid_eq(t: *const TUID, guid: &[u8; 16]) -> bool {
    // SAFETY: callers pass a valid 16-byte TUID or null.
    !t.is_null() && unsafe { &*(t as *const [u8; 16]) } == guid
}

/// `IHostApplication`: the context passed to `initialize` and `IPluginFactory3`.
#[derive(Default)]
pub(crate) struct HostApplication;

impl Class for HostApplication {
    type Interfaces = (IHostApplication,);
}

impl IHostApplicationTrait for HostApplication {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        if name.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: checked non-null; String128 is a fixed-size buffer.
        write_tchar(HOST_NAME, unsafe { &mut *name });
        kResultOk
    }

    unsafe fn createInstance(
        &self,
        cid: *mut TUID,
        iid: *mut TUID,
        obj: *mut *mut c_void,
    ) -> tresult {
        if obj.is_null() {
            return kInvalidArgument;
        }
        let ptr = if tuid_eq(cid, &IMessage::IID) && tuid_eq(iid, &IMessage::IID) {
            ComWrapper::new(HostMessage::default())
                .to_com_ptr::<IMessage>()
                .map(|p| p.into_raw().cast::<c_void>())
        } else if tuid_eq(cid, &IAttributeList::IID) && tuid_eq(iid, &IAttributeList::IID) {
            ComWrapper::new(HostAttributeList::default())
                .to_com_ptr::<IAttributeList>()
                .map(|p| p.into_raw().cast::<c_void>())
        } else {
            None
        };
        // SAFETY: `obj` checked non-null; ownership of one reference goes to the caller.
        unsafe { *obj = ptr.unwrap_or(std::ptr::null_mut()) };
        if ptr.is_some() {
            kResultOk
        } else {
            kNoInterface
        }
    }
}

#[derive(Clone)]
enum Attr {
    Int(i64),
    Float(f64),
    String(Vec<TChar>),
    Binary(Vec<u8>),
}

/// `IAttributeList` (host-created, for `IMessage`s between component and controller).
#[derive(Default)]
pub(crate) struct HostAttributeList {
    attrs: Mutex<HashMap<CString, Attr>>,
}

impl Class for HostAttributeList {
    type Interfaces = (IAttributeList,);
}

impl HostAttributeList {
    fn key(id: *const c_char) -> Option<CString> {
        // SAFETY: attribute ids are NUL-terminated C strings (or null).
        (!id.is_null()).then(|| unsafe { CStr::from_ptr(id) }.to_owned())
    }

    fn set(&self, id: *const c_char, value: Attr) -> tresult {
        let Some(key) = Self::key(id) else {
            return kInvalidArgument;
        };
        self.attrs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, value);
        kResultOk
    }

    fn with<R>(&self, id: *const c_char, f: impl FnOnce(&Attr) -> Option<R>) -> Option<R> {
        let key = Self::key(id)?;
        let attrs = self.attrs.lock().unwrap_or_else(|e| e.into_inner());
        attrs.get(&key).and_then(f)
    }
}

impl IAttributeListTrait for HostAttributeList {
    unsafe fn setInt(&self, id: FIDString, value: int64) -> tresult {
        self.set(id, Attr::Int(value))
    }

    unsafe fn getInt(&self, id: FIDString, value: *mut int64) -> tresult {
        match self.with(id, |a| match a {
            Attr::Int(v) => Some(*v),
            _ => None,
        }) {
            Some(v) if !value.is_null() => {
                // SAFETY: checked non-null out pointer.
                unsafe { *value = v };
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn setFloat(&self, id: FIDString, value: f64) -> tresult {
        self.set(id, Attr::Float(value))
    }

    unsafe fn getFloat(&self, id: FIDString, value: *mut f64) -> tresult {
        match self.with(id, |a| match a {
            Attr::Float(v) => Some(*v),
            _ => None,
        }) {
            Some(v) if !value.is_null() => {
                // SAFETY: checked non-null out pointer.
                unsafe { *value = v };
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn setString(&self, id: FIDString, string: *const TChar) -> tresult {
        if string.is_null() {
            return kInvalidArgument;
        }
        let mut v = Vec::new();
        let mut i = 0;
        // SAFETY: NUL-terminated UTF-16 string from the caller.
        while unsafe { *string.add(i) } != 0 {
            v.push(unsafe { *string.add(i) });
            i += 1;
        }
        self.set(id, Attr::String(v))
    }

    unsafe fn getString(&self, id: FIDString, string: *mut TChar, size_bytes: uint32) -> tresult {
        let cap = size_bytes as usize / std::mem::size_of::<TChar>();
        if string.is_null() || cap == 0 {
            return kInvalidArgument;
        }
        let Some(v) = self.with(id, |a| match a {
            Attr::String(v) => Some(v.clone()),
            _ => None,
        }) else {
            return kResultFalse;
        };
        let n = v.len().min(cap - 1);
        // SAFETY: the caller provides `size_bytes` writable bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(v.as_ptr(), string, n);
            *string.add(n) = 0;
        }
        kResultOk
    }

    unsafe fn setBinary(&self, id: FIDString, data: *const c_void, size: uint32) -> tresult {
        let bytes = if size == 0 {
            Vec::new()
        } else if data.is_null() {
            return kInvalidArgument;
        } else {
            // SAFETY: the caller provides `size` readable bytes.
            unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size as usize) }.to_vec()
        };
        self.set(id, Attr::Binary(bytes))
    }

    unsafe fn getBinary(
        &self,
        id: FIDString,
        data: *mut *const c_void,
        size: *mut uint32,
    ) -> tresult {
        if data.is_null() || size.is_null() {
            return kInvalidArgument;
        }
        // The pointer stays valid until the attribute is overwritten or the list dropped
        // (SDK semantics): the Vec's heap buffer does not move while it lives in the map.
        let Some((ptr, len)) = self.with(id, |a| match a {
            Attr::Binary(v) => Some((v.as_ptr(), v.len())),
            _ => None,
        }) else {
            return kResultFalse;
        };
        // SAFETY: checked non-null out pointers.
        unsafe {
            *data = ptr.cast();
            *size = len as uint32;
        }
        kResultOk
    }
}

/// `IMessage` (host-created through `IHostApplication::createInstance`).
pub(crate) struct HostMessage {
    id: Mutex<Option<CString>>,
    attributes: ComWrapper<HostAttributeList>,
}

impl Default for HostMessage {
    fn default() -> Self {
        Self {
            id: Mutex::new(None),
            attributes: ComWrapper::new(HostAttributeList::default()),
        }
    }
}

impl Class for HostMessage {
    type Interfaces = (IMessage,);
}

impl IMessageTrait for HostMessage {
    unsafe fn getMessageID(&self) -> FIDString {
        let id = self.id.lock().unwrap_or_else(|e| e.into_inner());
        // Valid until the next `setMessageID` (SDK semantics).
        id.as_ref().map_or(std::ptr::null(), |s| s.as_ptr())
    }

    unsafe fn setMessageID(&self, id: FIDString) {
        // SAFETY: NUL-terminated C string (or null).
        let v = (!id.is_null()).then(|| unsafe { CStr::from_ptr(id) }.to_owned());
        *self.id.lock().unwrap_or_else(|e| e.into_inner()) = v;
    }

    unsafe fn getAttributes(&self) -> *mut IAttributeList {
        // Not add-ref'd (SDK semantics: owned by the message).
        self.attributes
            .as_com_ref::<IAttributeList>()
            .map_or(std::ptr::null_mut(), |r| r.as_ptr())
    }
}

/// What the plugin's controller reported through `IComponentHandler(2)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Edit {
    Begin(ParamID),
    Perform(ParamID, ParamValue),
    End(ParamID),
}

/// `IComponentHandler` + `IComponentHandler2`.
#[derive(Default)]
pub(crate) struct ComponentHandler {
    pub edits: Mutex<Vec<Edit>>,
    /// OR of pending `RestartFlags`.
    pub restart: AtomicI32,
    pub dirty: AtomicBool,
}

impl Class for ComponentHandler {
    type Interfaces = (IComponentHandler, IComponentHandler2);
}

impl ComponentHandler {
    fn push(&self, e: Edit) -> tresult {
        self.edits.lock().unwrap_or_else(|e| e.into_inner()).push(e);
        kResultOk
    }

    pub fn take_edits(&self) -> Vec<Edit> {
        std::mem::take(&mut *self.edits.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn take_restart(&self) -> i32 {
        self.restart.swap(0, Ordering::AcqRel)
    }
}

impl IComponentHandlerTrait for ComponentHandler {
    unsafe fn beginEdit(&self, id: ParamID) -> tresult {
        self.push(Edit::Begin(id))
    }

    unsafe fn performEdit(&self, id: ParamID, value: ParamValue) -> tresult {
        self.push(Edit::Perform(id, value))
    }

    unsafe fn endEdit(&self, id: ParamID) -> tresult {
        self.push(Edit::End(id))
    }

    unsafe fn restartComponent(&self, flags: int32) -> tresult {
        self.restart.fetch_or(flags, Ordering::AcqRel);
        kResultOk
    }
}

impl IComponentHandler2Trait for ComponentHandler {
    unsafe fn setDirty(&self, state: TBool) -> tresult {
        if state != 0 {
            self.dirty.store(true, Ordering::Release);
        }
        kResultOk
    }

    unsafe fn requestOpenEditor(&self, _name: FIDString) -> tresult {
        kResultFalse
    }

    unsafe fn startGroupEdit(&self) -> tresult {
        kResultOk
    }

    unsafe fn finishGroupEdit(&self) -> tresult {
        kResultOk
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vst3::ComPtr;

    #[test]
    fn strings() {
        let mut buf = [0 as TChar; 6];
        write_tchar("héllo world", &mut buf);
        assert_eq!(read_tchar(&buf), "héllo");
        let c: Vec<c_char> = b"abc\0def".iter().map(|b| *b as c_char).collect();
        assert_eq!(read_char8(&c), "abc");
    }

    #[test]
    fn host_creates_messages_with_attributes() {
        let host = ComWrapper::new(HostApplication);
        let app = host.to_com_ptr::<IHostApplication>().unwrap();
        let mut cid = IMessage::IID.map(|b| b as c_char);
        let mut iid = cid;
        let mut obj = std::ptr::null_mut();
        unsafe {
            assert_eq!(app.createInstance(&mut cid, &mut iid, &mut obj), kResultOk);
            let msg = ComPtr::<IMessage>::from_raw(obj.cast()).unwrap();
            msg.setMessageID(c"hello".as_ptr());
            assert_eq!(CStr::from_ptr(msg.getMessageID()), c"hello");
            let attrs = vst3::ComRef::from_raw(msg.getAttributes()).unwrap();
            attrs.setInt(c"n".as_ptr(), 42);
            attrs.setFloat(c"f".as_ptr(), 0.5);
            let bin = [1u8, 2, 3];
            attrs.setBinary(c"b".as_ptr(), bin.as_ptr().cast(), 3);
            let wide: Vec<TChar> = "hi\0".encode_utf16().collect();
            attrs.setString(c"s".as_ptr(), wide.as_ptr());
            let (mut n, mut f) = (0, 0.0);
            assert_eq!(attrs.getInt(c"n".as_ptr(), &mut n), kResultOk);
            assert_eq!(attrs.getFloat(c"f".as_ptr(), &mut f), kResultOk);
            assert_eq!((n, f), (42, 0.5));
            assert_eq!(attrs.getInt(c"f".as_ptr(), &mut n), kResultFalse);
            let (mut p, mut len) = (std::ptr::null(), 0);
            assert_eq!(attrs.getBinary(c"b".as_ptr(), &mut p, &mut len), kResultOk);
            assert_eq!(
                std::slice::from_raw_parts(p.cast::<u8>(), len as usize),
                bin
            );
            let mut out = [0 as TChar; 8];
            assert_eq!(
                attrs.getString(c"s".as_ptr(), out.as_mut_ptr(), 16),
                kResultOk
            );
            assert_eq!(read_tchar(&out), "hi");
        }
        let mut other = [0 as c_char; 16];
        let mut obj = std::ptr::null_mut();
        let r = unsafe { app.createInstance(&mut other, &mut other.clone(), &mut obj) };
        assert_eq!(r, kNoInterface);
        assert!(obj.is_null());
    }
}
