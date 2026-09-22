//! Small audited FFI boundary compiled against the installed public C headers.
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr::NonNull;

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct Error {
    pub domain: c_int,
    pub code: c_int,
}
type Result<T> = std::result::Result<T, Error>;
#[repr(C)]
struct NativeDevice {
    udid: [c_char; 128],
    transport: c_int,
}
unsafe extern "C" {
    fn ac_list(out: *mut *mut NativeDevice, count: *mut c_int, error: *mut Error) -> c_int;
    fn ac_free(ptr: *mut c_void);
    fn ac_open(udid: *const c_char, transport: c_int, error: *mut Error) -> *mut c_void;
    fn ac_close(session: *mut c_void);
    fn ac_version(
        session: *mut c_void,
        out: *mut c_char,
        capacity: u32,
        error: *mut Error,
    ) -> c_int;
    fn ac_start(
        session: *mut c_void,
        name: *const c_char,
        afc: c_int,
        tls: *mut c_int,
        error: *mut Error,
    ) -> *mut c_void;
    fn ac_service_free(service: *mut c_void);
    fn ac_receive(
        service: *mut c_void,
        buf: *mut c_char,
        size: u32,
        received: *mut u32,
        timeout: u32,
        error: *mut Error,
    ) -> c_int;
    fn ac_send(
        service: *mut c_void,
        buf: *const c_char,
        size: u32,
        sent: *mut u32,
        timeout: u32,
        error: *mut Error,
    ) -> c_int;
    fn ac_afc_free(afc: *mut c_void);
    fn ac_afc_rename(
        afc: *mut c_void,
        source: *const c_char,
        target: *const c_char,
        error: *mut Error,
    ) -> c_int;
    fn ac_dictionary_free(dict: *mut *mut c_char);
    fn ac_afc_list(
        afc: *mut c_void,
        path: *const c_char,
        out: *mut *mut *mut c_char,
        error: *mut Error,
    ) -> c_int;
    fn ac_afc_info(
        afc: *mut c_void,
        path: *const c_char,
        out: *mut *mut *mut c_char,
        error: *mut Error,
    ) -> c_int;
    fn ac_afc_mkdir(afc: *mut c_void, path: *const c_char, error: *mut Error) -> c_int;
    fn ac_afc_remove(afc: *mut c_void, path: *const c_char, error: *mut Error) -> c_int;
    fn ac_afc_open(
        afc: *mut c_void,
        path: *const c_char,
        write: c_int,
        handle: *mut u64,
        error: *mut Error,
    ) -> c_int;
    fn ac_afc_file_close(afc: *mut c_void, handle: u64, error: *mut Error) -> c_int;
    fn ac_afc_read(
        afc: *mut c_void,
        handle: u64,
        buf: *mut c_char,
        size: u32,
        read: *mut u32,
        error: *mut Error,
    ) -> c_int;
    fn ac_afc_write(
        afc: *mut c_void,
        handle: u64,
        buf: *const c_char,
        size: u32,
        written: *mut u32,
        error: *mut Error,
    ) -> c_int;
}
fn cstring(s: &str) -> Result<CString> {
    CString::new(s).map_err(|_| Error {
        domain: 6,
        code: -1,
    })
}
fn check(code: c_int, error: Error) -> Result<()> {
    if code == 0 { Ok(()) } else { Err(error) }
}
fn size(len: usize) -> Result<u32> {
    u32::try_from(len).map_err(|_| Error {
        domain: 6,
        code: -1,
    })
}

pub fn list() -> Result<Vec<(String, i32)>> {
    let mut ptr = std::ptr::null_mut();
    let mut count = 0;
    let mut error = Error::default();
    // SAFETY: out pointers are valid; C allocates a bounded count of NativeDevice records.
    check(unsafe { ac_list(&mut ptr, &mut count, &mut error) }, error)?;
    let mut result = Vec::new();
    // SAFETY: ac_list guarantees count initialized records with terminated UDIDs; release once.
    unsafe {
        for row in std::slice::from_raw_parts(ptr, count as usize) {
            result.push((
                CStr::from_ptr(row.udid.as_ptr())
                    .to_string_lossy()
                    .into_owned(),
                row.transport,
            ));
        }
        ac_free(ptr.cast());
    }
    Ok(result)
}

pub struct Session(NonNull<c_void>);
impl Session {
    pub fn open(udid: &str, transport: i32) -> Result<Self> {
        if ![1, 2].contains(&transport) {
            return Err(Error {
                domain: 6,
                code: -1,
            });
        }
        let udid = cstring(udid)?;
        let mut error = Error::default();
        // SAFETY: input remains valid during call; unique returned handle is owned by Self.
        NonNull::new(unsafe { ac_open(udid.as_ptr(), transport, &mut error) })
            .map(Self)
            .ok_or(error)
    }
    pub fn version(&mut self) -> Result<String> {
        let mut buf = [0 as c_char; 64];
        let mut error = Error::default();
        // SAFETY: live session and writable capacity; C guarantees termination on success.
        check(
            // SAFETY: live session and writable capacity; C guarantees termination on success.
            unsafe {
                ac_version(
                    self.0.as_ptr(),
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &mut error,
                )
            },
            error,
        )?;
        // SAFETY: successful ac_version wrote a NUL-terminated string inside buf.
        Ok(unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned())
    }
    fn start(&mut self, name: &str, afc: bool) -> Result<(NonNull<c_void>, bool)> {
        let name = cstring(name)?;
        let mut tls = 0;
        let mut error = Error::default();
        // SAFETY: live session and valid output pointers; returned service has unique ownership.
        let ptr = NonNull::new(unsafe {
            ac_start(
                self.0.as_ptr(),
                name.as_ptr(),
                afc.into(),
                &mut tls,
                &mut error,
            )
        })
        .ok_or(error)?;
        Ok((ptr, tls != 0))
    }
    pub fn service(mut self, name: &str) -> Result<Service> {
        let (ptr, tls) = self.start(name, false)?;
        Ok(Service {
            ptr,
            tls,
            _session: self,
        })
    }
    pub fn afc(mut self) -> Result<Afc> {
        let (ptr, tls) = self.start("com.apple.afc", true)?;
        Ok(Afc {
            ptr,
            tls,
            _session: self,
        })
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: this owns the unique session and all child clients have already been released.
        unsafe { ac_close(self.0.as_ptr()) }
    }
}
pub struct Service {
    ptr: NonNull<c_void>,
    pub tls: bool,
    _session: Session,
}
impl Service {
    pub fn send(&mut self, buf: &[u8], timeout_ms: u32) -> Result<usize> {
        let len = size(buf.len())?;
        if len == 0 {
            return Ok(0);
        }
        let mut sent = 0;
        let mut error = Error::default();
        // SAFETY: uniquely owned live connection, readable len-byte buffer and valid output pointers.
        let code = unsafe {
            ac_send(
                self.ptr.as_ptr(),
                buf.as_ptr().cast(),
                len,
                &mut sent,
                timeout_ms,
                &mut error,
            )
        };
        check(code, error)?;
        Ok(sent as usize)
    }
    pub fn receive(&mut self, buf: &mut [u8], timeout_ms: u32) -> Result<usize> {
        let len = size(buf.len())?;
        let mut received = 0;
        let mut error = Error::default();
        if len == 0 {
            return Ok(0);
        }
        // SAFETY: uniquely borrowed live service and writable buffer of len bytes.
        check(
            // SAFETY: uniquely borrowed live service and writable buffer of len bytes.
            unsafe {
                ac_receive(
                    self.ptr.as_ptr(),
                    buf.as_mut_ptr().cast(),
                    len,
                    &mut received,
                    timeout_ms,
                    &mut error,
                )
            },
            error,
        )?;
        Ok(received as usize)
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        // SAFETY: free owned service exactly once, before _session is dropped.
        unsafe { ac_service_free(self.ptr.as_ptr()) }
    }
}
pub struct Afc {
    ptr: NonNull<c_void>,
    pub tls: bool,
    _session: Session,
}
impl Afc {
    pub fn rename(&mut self, source: &str, target: &str) -> Result<()> {
        let source = cstring(source)?;
        let target = cstring(target)?;
        let mut error = Error::default();
        // SAFETY: live exclusively borrowed client and valid C strings/output pointer.
        let code = unsafe {
            ac_afc_rename(
                self.ptr.as_ptr(),
                source.as_ptr(),
                target.as_ptr(),
                &mut error,
            )
        };
        check(code, error)
    }
    fn dictionary(&mut self, path: &str, info: bool) -> Result<Vec<String>> {
        let path = cstring(path)?;
        let mut ptr = std::ptr::null_mut();
        let mut error = Error::default();
        // SAFETY: live client and valid path/output pointers. AFC returns a NULL-terminated list.
        let code = unsafe {
            if info {
                ac_afc_info(self.ptr.as_ptr(), path.as_ptr(), &mut ptr, &mut error)
            } else {
                ac_afc_list(self.ptr.as_ptr(), path.as_ptr(), &mut ptr, &mut error)
            }
        };
        let mut out = Vec::new();
        // SAFETY: non-null dictionary is owned by us; all strings are C-terminated; free on errors too.
        unsafe {
            if !ptr.is_null() {
                if code == 0 {
                    let mut index = 0;
                    while !(*ptr.add(index)).is_null() {
                        out.push(
                            CStr::from_ptr(*ptr.add(index))
                                .to_string_lossy()
                                .into_owned(),
                        );
                        index += 1;
                    }
                }
                ac_dictionary_free(ptr);
            }
        }
        check(code, error)?;
        Ok(out)
    }
    pub fn list(&mut self, path: &str) -> Result<Vec<String>> {
        self.dictionary(path, false)
    }
    pub fn info(&mut self, path: &str) -> Result<Vec<String>> {
        self.dictionary(path, true)
    }
    pub fn mkdir(&mut self, path: &str) -> Result<()> {
        let path = cstring(path)?;
        let mut error = Error::default();
        // SAFETY: live client and valid C string for duration of call.
        check(
            // SAFETY: live client and valid C string for duration of call.
            unsafe { ac_afc_mkdir(self.ptr.as_ptr(), path.as_ptr(), &mut error) },
            error,
        )
    }
    pub fn remove(&mut self, path: &str) -> Result<()> {
        let path = cstring(path)?;
        let mut error = Error::default();
        // SAFETY: live client and valid C string for duration of call.
        check(
            // SAFETY: live client and valid C string for duration of call.
            unsafe { ac_afc_remove(self.ptr.as_ptr(), path.as_ptr(), &mut error) },
            error,
        )
    }
    pub fn open(&mut self, path: &str, write: bool) -> Result<File<'_>> {
        let path = cstring(path)?;
        let mut handle = 0;
        let mut error = Error::default();
        // SAFETY: valid pointers; exclusive borrow keeps client alive until File is closed.
        check(
            // SAFETY: valid pointers; exclusive borrow keeps client alive until File is closed.
            unsafe {
                ac_afc_open(
                    self.ptr.as_ptr(),
                    path.as_ptr(),
                    write.into(),
                    &mut handle,
                    &mut error,
                )
            },
            error,
        )?;
        Ok(File {
            client: self,
            handle: Some(handle),
        })
    }
}
impl Drop for Afc {
    fn drop(&mut self) {
        // SAFETY: free unique client before its session.
        unsafe { ac_afc_free(self.ptr.as_ptr()) }
    }
}
pub struct File<'a> {
    client: &'a mut Afc,
    handle: Option<u64>,
}
impl File<'_> {
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let len = size(buf.len())?;
        let mut read = 0;
        let mut error = Error::default();
        // SAFETY: File holds exclusive live client and open handle, buffer capacity equals len.
        check(
            // SAFETY: File holds exclusive live client and open handle, buffer capacity equals len.
            unsafe {
                ac_afc_read(
                    self.client.ptr.as_ptr(),
                    self.handle.unwrap(),
                    buf.as_mut_ptr().cast(),
                    len,
                    &mut read,
                    &mut error,
                )
            },
            error,
        )?;
        Ok(read as usize)
    }
    pub fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let len = size(buf.len())?;
        let mut written = 0;
        let mut error = Error::default();
        // SAFETY: File holds exclusive live client and open handle; buffer readable for len bytes.
        check(
            // SAFETY: File holds exclusive live client and open handle; buffer readable for len bytes.
            unsafe {
                ac_afc_write(
                    self.client.ptr.as_ptr(),
                    self.handle.unwrap(),
                    buf.as_ptr().cast(),
                    len,
                    &mut written,
                    &mut error,
                )
            },
            error,
        )?;
        Ok(written as usize)
    }
    pub fn close(mut self) -> Result<()> {
        self.close_inner()
    }
    fn close_inner(&mut self) -> Result<()> {
        if let Some(handle) = self.handle.take() {
            let mut error = Error::default();
            // SAFETY: owned open handle is removed before calling, so it cannot be closed twice.
            check(
                // SAFETY: owned open handle is removed before calling, so it cannot be closed twice.
                unsafe { ac_afc_file_close(self.client.ptr.as_ptr(), handle, &mut error) },
                error,
            )?;
        }
        Ok(())
    }
}
impl Drop for File<'_> {
    fn drop(&mut self) {
        let _ = self.close_inner();
    }
}
