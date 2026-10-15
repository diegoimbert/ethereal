//! `IBStream` over an in-memory byte buffer, for component/controller state.

use std::ffi::c_void;
use std::sync::Mutex;

use vst3::Steinberg::IBStream_::IStreamSeekMode_::{kIBSeekCur, kIBSeekEnd, kIBSeekSet};
use vst3::Steinberg::{
    IBStream, IBStreamTrait, int32, int64, kInvalidArgument, kResultOk, tresult,
};
use vst3::{Class, ComWrapper};

#[derive(Default)]
struct Inner {
    data: Vec<u8>,
    pos: usize,
}

/// A growable, seekable memory stream (the SDK's `MemoryStream`).
#[derive(Default)]
pub(crate) struct MemoryStream {
    inner: Mutex<Inner>,
}

impl Class for MemoryStream {
    type Interfaces = (IBStream,);
}

impl MemoryStream {
    pub fn new() -> ComWrapper<Self> {
        ComWrapper::new(Self::default())
    }

    /// A stream positioned at the start of `data`.
    pub fn from_bytes(data: &[u8]) -> ComWrapper<Self> {
        ComWrapper::new(Self {
            inner: Mutex::new(Inner {
                data: data.to_vec(),
                pos: 0,
            }),
        })
    }

    pub fn bytes(&self) -> Vec<u8> {
        self.lock().data.clone()
    }

    pub fn rewind(&self) {
        self.lock().pos = 0;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl IBStreamTrait for MemoryStream {
    unsafe fn read(&self, buffer: *mut c_void, num_bytes: int32, read: *mut int32) -> tresult {
        if num_bytes < 0 || (buffer.is_null() && num_bytes > 0) {
            return kInvalidArgument;
        }
        let mut s = self.lock();
        let start = s.pos.min(s.data.len());
        let n = (num_bytes as usize).min(s.data.len() - start);
        if n > 0 {
            // SAFETY: the caller provides `num_bytes` writable bytes at `buffer`.
            unsafe { std::ptr::copy_nonoverlapping(s.data[start..].as_ptr(), buffer.cast(), n) };
        }
        s.pos = start + n;
        if !read.is_null() {
            // SAFETY: optional out pointer from the caller.
            unsafe { *read = n as int32 };
        }
        kResultOk
    }

    unsafe fn write(&self, buffer: *mut c_void, num_bytes: int32, written: *mut int32) -> tresult {
        if num_bytes < 0 || (buffer.is_null() && num_bytes > 0) {
            return kInvalidArgument;
        }
        let n = num_bytes as usize;
        let mut s = self.lock();
        let pos = s.pos;
        if s.data.len() < pos + n {
            s.data.resize(pos + n, 0);
        }
        if n > 0 {
            // SAFETY: the caller provides `num_bytes` readable bytes at `buffer`.
            let src = unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), n) };
            s.data[pos..pos + n].copy_from_slice(src);
        }
        s.pos = pos + n;
        if !written.is_null() {
            // SAFETY: optional out pointer from the caller.
            unsafe { *written = n as int32 };
        }
        kResultOk
    }

    unsafe fn seek(&self, pos: int64, mode: int32, result: *mut int64) -> tresult {
        let mut s = self.lock();
        let base = if mode == kIBSeekSet as int32 {
            0
        } else if mode == kIBSeekCur as int32 {
            s.pos as i64
        } else if mode == kIBSeekEnd as int32 {
            s.data.len() as i64
        } else {
            return kInvalidArgument;
        };
        let Some(new) = base.checked_add(pos).filter(|p| *p >= 0) else {
            return kInvalidArgument;
        };
        s.pos = new as usize;
        if !result.is_null() {
            // SAFETY: optional out pointer from the caller.
            unsafe { *result = new };
        }
        kResultOk
    }

    unsafe fn tell(&self, pos: *mut int64) -> tresult {
        if pos.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: checked non-null above.
        unsafe { *pos = self.lock().pos as int64 };
        kResultOk
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_write_seek() {
        let s = MemoryStream::new();
        let p = s.to_com_ptr::<IBStream>().unwrap();
        let mut data = *b"hello world";
        let mut n = 0;
        unsafe {
            assert_eq!(p.write(data.as_mut_ptr().cast(), 11, &mut n), kResultOk);
            assert_eq!(n, 11);
            let mut at = 0;
            assert_eq!(p.seek(-5, kIBSeekEnd as i32, &mut at), kResultOk);
            assert_eq!(at, 6);
            let mut out = [0u8; 16];
            assert_eq!(p.read(out.as_mut_ptr().cast(), 16, &mut n), kResultOk);
            assert_eq!(&out[..n as usize], b"world");
            assert_eq!(p.seek(-1, kIBSeekSet as i32, &mut at), kInvalidArgument);
            let mut t = 0;
            assert_eq!(p.tell(&mut t), kResultOk);
            assert_eq!(t, 11);
            // Seeking past the end then writing zero-fills.
            p.seek(2, kIBSeekCur as i32, std::ptr::null_mut());
            let mut x = *b"!";
            p.write(x.as_mut_ptr().cast(), 1, std::ptr::null_mut());
        }
        assert_eq!(s.bytes(), b"hello world\0\0!");
    }
}
