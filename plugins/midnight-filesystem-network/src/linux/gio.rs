//! Audited dynamically loaded GIO boundary. No GTK, shell, credential storage or worker pool.
//! Calls run on the caller's bounded backend worker. Async callbacks are drained before return.
use super::{
    backend::{Backend, Entry},
    uri::MAX_URI_BYTES,
};
use crate::{
    adapter::{Context, NodeKind, Target},
    api::{ErrorCode, Failure},
};
use libloading::Library;
use std::{
    ffi::{CString, c_char, c_int, c_void},
    ptr,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

type Object = *mut c_void;
type Callback = unsafe extern "C" fn(Object, Object, Object);
#[repr(C)]
struct GError {
    domain: u32,
    code: c_int,
    message: *mut c_char,
}
#[repr(C)]
struct GList {
    data: Object,
    next: *mut GList,
    previous: *mut GList,
}
type PointerFinish = unsafe extern "C" fn(Object, Object, *mut *mut GError) -> Object;
type BooleanFinish = unsafe extern "C" fn(Object, Object, *mut *mut GError) -> c_int;

macro_rules! symbols {
    ($($name:ident: $ty:ty => $library:literal),+ $(,)?) => {
        struct Symbols { $($name: $ty,)+ _libraries: [Library; 3] }
        impl Symbols {
            fn load() -> Result<Self, Failure> {
                // SAFETY: fixed system SONAMEs only; retained libraries outlive all function pointers.
                unsafe {
                    let glib = Library::new("libglib-2.0.so.0").map_err(|_| unavailable())?;
                    let gobject = Library::new("libgobject-2.0.so.0").map_err(|_| unavailable())?;
                    let gio = Library::new("libgio-2.0.so.0").map_err(|_| unavailable())?;
                    let libraries = [gio, gobject, glib];
                    Ok(Self { $($name: *libraries[$library].get::<$ty>(concat!(stringify!($name), "\0").as_bytes()).map_err(|_| unavailable())?,)+ _libraries: libraries })
                }
            }
        }
    }
}
symbols! {
    g_file_new_for_uri: unsafe extern "C" fn(*const c_char) -> Object => 0,
    g_file_get_path: unsafe extern "C" fn(Object) -> *mut c_char => 0,
    g_file_enumerate_children_async: unsafe extern "C" fn(Object,*const c_char,c_int,c_int,Object,Callback,Object) -> () => 0,
    g_file_enumerate_children_finish: PointerFinish => 0,
    g_file_enumerator_next_files_async: unsafe extern "C" fn(Object,c_int,c_int,Object,Callback,Object) -> () => 0,
    g_file_enumerator_next_files_finish: PointerFinish => 0,
    g_file_enumerator_close: unsafe extern "C" fn(Object,Object,*mut *mut GError) -> c_int => 0,
    g_file_info_get_attribute_string: unsafe extern "C" fn(Object,*const c_char) -> *const c_char => 0,
    g_file_info_get_display_name: unsafe extern "C" fn(Object) -> *const c_char => 0,
    g_file_info_get_file_type: unsafe extern "C" fn(Object) -> c_int => 0,
    g_file_mount_enclosing_volume: unsafe extern "C" fn(Object,c_int,Object,Object,Callback,Object) -> () => 0,
    g_file_mount_enclosing_volume_finish: BooleanFinish => 0,
    g_cancellable_new: unsafe extern "C" fn() -> Object => 0,
    g_cancellable_cancel: unsafe extern "C" fn(Object) -> () => 0,
    g_mount_operation_new: unsafe extern "C" fn() -> Object => 0,
    g_mount_operation_reply: unsafe extern "C" fn(Object,c_int) -> () => 0,
    g_mount_operation_set_password_save: unsafe extern "C" fn(Object,c_int) -> () => 0,
    g_io_error_quark: unsafe extern "C" fn() -> u32 => 0,
    g_object_unref: unsafe extern "C" fn(Object) -> () => 1,
    g_signal_connect_data: unsafe extern "C" fn(Object,*const c_char,unsafe extern "C" fn(),Object,Option<unsafe extern "C" fn(Object,Object)>,u32) -> usize => 1,
    g_main_context_new: unsafe extern "C" fn() -> Object => 2,
    g_main_context_unref: unsafe extern "C" fn(Object) -> () => 2,
    g_main_context_push_thread_default: unsafe extern "C" fn(Object) -> () => 2,
    g_main_context_pop_thread_default: unsafe extern "C" fn(Object) -> () => 2,
    g_main_context_iteration: unsafe extern "C" fn(Object,c_int) -> c_int => 2,
    g_error_free: unsafe extern "C" fn(*mut GError) -> () => 2,
    g_free: unsafe extern "C" fn(Object) -> () => 2,
    g_list_free_full: unsafe extern "C" fn(*mut GList,unsafe extern "C" fn(Object)) -> () => 2,
}
fn unavailable() -> Failure {
    Failure::new(
        ErrorCode::Unavailable,
        "GIO/GVfs network support is unavailable in this session.",
    )
}
fn runtime() -> Result<Arc<Symbols>, Failure> {
    static SYMBOLS: OnceLock<Result<Arc<Symbols>, Failure>> = OnceLock::new();
    SYMBOLS
        .get_or_init(|| Symbols::load().map(Arc::new))
        .clone()
}
pub(super) struct Gio;
pub struct Cursor {
    api: Arc<Symbols>,
    context: MainContext,
    enumerator: OwnedObject,
}
// SAFETY: the cursor is exclusively borrowed for each operation. No operation or callback
// remains pending between calls; the private GMainContext is acquired on the calling thread.
// GIO references and the context may move between workers, never be accessed concurrently.
unsafe impl Send for Cursor {}

struct OwnedObject {
    api: Arc<Symbols>,
    pointer: Object,
}
impl OwnedObject {
    fn new(api: &Arc<Symbols>, pointer: Object) -> Result<Self, Failure> {
        if pointer.is_null() {
            Err(Failure::new(
                ErrorCode::ResourceLimit,
                "GIO could not allocate an operation resource.",
            ))
        } else {
            Ok(Self {
                api: api.clone(),
                pointer,
            })
        }
    }
}
impl Drop for OwnedObject {
    fn drop(&mut self) {
        // SAFETY: this guard owns exactly one non-null GObject reference.
        unsafe {
            (self.api.g_object_unref)(self.pointer);
        }
    }
}
struct MainContext {
    api: Arc<Symbols>,
    pointer: Object,
}
impl MainContext {
    fn new(api: &Arc<Symbols>) -> Result<Self, Failure> {
        // SAFETY: no inputs; ownership transfers on success.
        let pointer = unsafe { (api.g_main_context_new)() };
        if pointer.is_null() {
            return Err(Failure::new(
                ErrorCode::ResourceLimit,
                "GIO context allocation failed.",
            ));
        }
        Ok(Self {
            api: api.clone(),
            pointer,
        })
    }
    fn enter(&self) -> ThreadDefault<'_> {
        // SAFETY: cursor calls are serialized, and every previous operation drained callbacks.
        unsafe {
            (self.api.g_main_context_push_thread_default)(self.pointer);
        }
        ThreadDefault(self)
    }
}
struct ThreadDefault<'a>(&'a MainContext);
impl Drop for ThreadDefault<'_> {
    fn drop(&mut self) {
        // SAFETY: paired with enter on this same thread and live context.
        unsafe {
            (self.0.api.g_main_context_pop_thread_default)(self.0.pointer);
        }
    }
}
impl Drop for MainContext {
    fn drop(&mut self) {
        // SAFETY: owned context, no pending operation or active thread-default guard.
        unsafe {
            (self.api.g_main_context_unref)(self.pointer);
        }
    }
}
impl Drop for Cursor {
    fn drop(&mut self) {
        let _entered = self.context.enter();
        let mut error = ptr::null_mut();
        // SAFETY: live enumerator, no outstanding requests. Cleanup remains on the backend caller.
        unsafe {
            (self.api.g_file_enumerator_close)(
                self.enumerator.pointer,
                ptr::null_mut(),
                &mut error,
            );
            if !error.is_null() {
                (self.api.g_error_free)(error);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Finish {
    Object(PointerFinish),
    List(PointerFinish),
    Boolean(BooleanFinish),
}
struct Completion {
    finish: Finish,
    result: Option<(Object, *mut GError)>,
    succeeded: bool,
}
unsafe extern "C" fn completed(source: Object, result: Object, data: Object) {
    // SAFETY: run keeps this allocation live, and GIO invokes its callback once on its context.
    let completion = unsafe { &mut *data.cast::<Completion>() };
    let mut error = ptr::null_mut();
    // SAFETY: finish function is paired with the initiated operation and callback's result.
    let pointer = unsafe {
        match completion.finish {
            Finish::Object(finish) | Finish::List(finish) => finish(source, result, &mut error),
            Finish::Boolean(finish) => {
                completion.succeeded = finish(source, result, &mut error) != 0;
                ptr::null_mut()
            }
        }
    };
    completion.result = Some((pointer, error));
}
struct Finished {
    api: Arc<Symbols>,
    finish: Finish,
    pointer: Object,
}
impl Finished {
    fn take_object(mut self) -> Result<OwnedObject, Failure> {
        let object = OwnedObject::new(&self.api, self.pointer)?;
        self.pointer = ptr::null_mut();
        Ok(object)
    }
}
impl Drop for Finished {
    fn drop(&mut self) {
        if self.pointer.is_null() {
            return;
        }
        // SAFETY: finish transfers ownership, with the matching result kind tracked here.
        unsafe {
            match self.finish {
                Finish::Object(_) => (self.api.g_object_unref)(self.pointer),
                Finish::List(_) => {
                    (self.api.g_list_free_full)(self.pointer.cast(), self.api.g_object_unref)
                }
                Finish::Boolean(_) => {}
            }
        }
    }
}
fn run(
    api: &Arc<Symbols>,
    main: &MainContext,
    context: &Context<'_>,
    finish: Finish,
    start: impl FnOnce(Object, Callback, Object),
) -> Result<Finished, Failure> {
    context.check()?;
    let _entered = main.enter();
    // SAFETY: fresh owned cancellable.
    let cancel = OwnedObject::new(api, unsafe { (api.g_cancellable_new)() })?;
    let mut completion = Box::new(Completion {
        finish,
        result: None,
        succeeded: true,
    });
    start(
        cancel.pointer,
        completed,
        (&mut *completion as *mut Completion).cast(),
    );
    let mut stopped = None;
    while completion.result.is_none() {
        if stopped.is_none()
            && let Err(error) = context.check()
        {
            stopped = Some(error);
            // SAFETY: cancellable remains live until callback completion.
            unsafe {
                (api.g_cancellable_cancel)(cancel.pointer);
            }
        }
        // SAFETY: dispatch only this operation's private acquired context; never block in poll.
        let dispatched = unsafe { (api.g_main_context_iteration)(main.pointer, 0) };
        if dispatched == 0 && completion.result.is_none() {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let (pointer, error) = completion
        .result
        .take()
        .ok_or_else(|| Failure::new(ErrorCode::Io, "GIO completion was lost."))?;
    let finished = Finished {
        api: api.clone(),
        finish,
        pointer,
    };
    let failure = if error.is_null() {
        None
    } else {
        Some(owned_error(api, error))
    };
    if let Some(error) = stopped {
        return Err(error);
    }
    context.check()?;
    if let Some(error) = failure {
        return Err(error);
    }
    if !completion.succeeded {
        return Err(Failure::new(
            ErrorCode::Io,
            "GIO did not complete the requested operation.",
        ));
    }
    Ok(finished)
}
fn owned_error(api: &Symbols, error: *mut GError) -> Failure {
    // SAFETY: non-null owned GError from a completed GIO operation; read before freeing.
    let (domain, code) = unsafe { ((*error).domain, (*error).code) };
    // SAFETY: error domain accessor has no inputs.
    let gio = unsafe { (api.g_io_error_quark)() };
    let mapped = if domain != gio {
        ErrorCode::Unavailable
    } else {
        match code {
            1 => ErrorCode::NotFound,
            13 | 35 => ErrorCode::Invalid,
            14 => ErrorCode::PermissionDenied,
            15 | 32 => ErrorCode::Unavailable,
            16 => ErrorCode::MountRequired,
            17 => ErrorCode::Busy,
            19 => ErrorCode::Cancelled,
            20 | 26 => ErrorCode::Busy,
            24 => ErrorCode::TimedOut,
            28 | 37..=39 | 44 | 45 => ErrorCode::Offline,
            31 | 46 => ErrorCode::ResourceLimit,
            _ => ErrorCode::Io,
        }
    };
    // Never return the native diagnostic: it can contain URI userinfo or server-provided text.
    let failure = Failure {
        code: mapped,
        os_code: Some(code),
        message: format!("GIO network operation failed ({mapped:?})."),
    };
    // SAFETY: transfer-owned error released exactly once.
    unsafe {
        (api.g_error_free)(error);
    }
    failure
}
fn file(api: &Arc<Symbols>, uri: &[u8]) -> Result<OwnedObject, Failure> {
    let uri = CString::new(uri)
        .map_err(|_| Failure::new(ErrorCode::Invalid, "Invalid network location."))?;
    // SAFETY: NUL-terminated validated URI is borrowed for the call; returned GFile is owned.
    OwnedObject::new(api, unsafe { (api.g_file_new_for_uri)(uri.as_ptr()) })
}
unsafe fn bounded_bytes(pointer: *const c_char) -> Result<Vec<u8>, Failure> {
    if pointer.is_null() {
        return Ok(Vec::new());
    }
    let mut length = 0;
    // SAFETY: caller supplies a GIO-owned NUL-terminated string, live for this call.
    while length <= MAX_URI_BYTES && unsafe { *pointer.add(length) } != 0 {
        length += 1;
    }
    if length > MAX_URI_BYTES {
        return Err(Failure::new(
            ErrorCode::ResourceLimit,
            "GIO text exceeds the admission budget.",
        ));
    }
    // SAFETY: preceding scan established the readable length, excluding its terminator.
    Ok(unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), length) }.to_vec())
}

struct Challenge {
    reply: unsafe extern "C" fn(Object, c_int),
    password: AtomicBool,
}
unsafe extern "C" fn password_requested(
    operation: Object,
    _message: *const c_char,
    _user: *const c_char,
    _domain: *const c_char,
    _flags: u32,
    data: Object,
) {
    // SAFETY: activation owns this state until the operation has completed and signals disconnected.
    let challenge = unsafe { &*data.cast::<Challenge>() };
    challenge.password.store(true, Ordering::Release);
    // SAFETY: live emitting operation; ABORTED requires no UI or credential values.
    unsafe {
        (challenge.reply)(operation, 1);
    }
}
unsafe extern "C" fn question_requested(
    operation: Object,
    _message: *const c_char,
    _choices: *const *const c_char,
    data: Object,
) {
    // SAFETY: same owned activation state as password_requested.
    let challenge = unsafe { &*data.cast::<Challenge>() };
    // SAFETY: reject interaction rather than presenting an implicit question.
    unsafe {
        (challenge.reply)(operation, 1);
    }
}

impl Backend for Gio {
    type Cursor = Cursor;
    fn begin(&self, uri: &[u8], context: &Context<'_>) -> Result<Cursor, Failure> {
        context.check()?;
        let api = runtime()?;
        let main = MainContext::new(&api)?;
        let file = file(&api, uri)?;
        let attributes = c"standard::type,standard::display-name,standard::target-uri";
        let result = run(
            &api,
            &main,
            context,
            Finish::Object(api.g_file_enumerate_children_finish),
            |cancel, callback, data| {
                // SAFETY: all borrowed inputs and callback state remain live until run drains completion.
                unsafe {
                    (api.g_file_enumerate_children_async)(
                        file.pointer,
                        attributes.as_ptr(),
                        0,
                        0,
                        cancel,
                        callback,
                        data,
                    );
                }
            },
        )?;
        let enumerator = result.take_object()?;
        Ok(Cursor {
            api,
            context: main,
            enumerator,
        })
    }
    fn next(&self, cursor: &mut Cursor, context: &Context<'_>) -> Result<Option<Entry>, Failure> {
        let api = &cursor.api;
        let result = run(
            api,
            &cursor.context,
            context,
            Finish::List(api.g_file_enumerator_next_files_finish),
            |cancel, callback, data| {
                // SAFETY: exclusive cursor means no overlapping operations on its enumerator.
                unsafe {
                    (api.g_file_enumerator_next_files_async)(
                        cursor.enumerator.pointer,
                        1,
                        0,
                        cancel,
                        callback,
                        data,
                    );
                }
            },
        )?;
        if result.pointer.is_null() {
            return Ok(None);
        }
        // SAFETY: next_files_finish returned an owned GList of live GFileInfo objects.
        let info = unsafe { (*result.pointer.cast::<GList>()).data };
        if info.is_null() {
            return Err(Failure::new(
                ErrorCode::Io,
                "GIO returned an invalid discovery record.",
            ));
        }
        // SAFETY: info remains owned by result; attribute strings are borrowed until result drops.
        let (uri, display) = unsafe {
            let kind = (api.g_file_info_get_file_type)(info);
            if !matches!(kind, 2 | 5 | 6) {
                return Ok(Some(Entry {
                    uri: Vec::new(),
                    display: String::new(),
                }));
            }
            let uri = bounded_bytes((api.g_file_info_get_attribute_string)(
                info,
                c"standard::target-uri".as_ptr(),
            ))?;
            let display = bounded_bytes((api.g_file_info_get_display_name)(info))?;
            (uri, display)
        };
        let display = String::from_utf8(display)
            .map_err(|_| Failure::new(ErrorCode::Invalid, "GIO returned a non-UTF-8 caption."))?;
        Ok(Some(Entry { uri, display }))
    }
    fn activate(&self, target: &Target, context: &Context<'_>) -> Result<Option<Vec<u8>>, Failure> {
        context.check()?;
        let api = runtime()?;
        let main = MainContext::new(&api)?;
        let file = file(&api, &target.key)?;
        let mut challenge = Box::new(Challenge {
            reply: api.g_mount_operation_reply,
            password: AtomicBool::new(false),
        });
        // SAFETY: new plain GMountOperation, never GtkMountOperation or a UI proxy.
        let operation = OwnedObject::new(&api, unsafe { (api.g_mount_operation_new)() })?;
        // SAFETY: signal signatures match GMountOperation documentation; state outlives operation.
        unsafe {
            (api.g_mount_operation_set_password_save)(operation.pointer, 0);
            let data = (&mut *challenge as *mut Challenge).cast();
            (api.g_signal_connect_data)(
                operation.pointer,
                c"ask-password".as_ptr(),
                std::mem::transmute::<
                    unsafe extern "C" fn(
                        Object,
                        *const c_char,
                        *const c_char,
                        *const c_char,
                        u32,
                        Object,
                    ),
                    unsafe extern "C" fn(),
                >(password_requested),
                data,
                None,
                0,
            );
            (api.g_signal_connect_data)(
                operation.pointer,
                c"ask-question".as_ptr(),
                std::mem::transmute::<
                    unsafe extern "C" fn(Object, *const c_char, *const *const c_char, Object),
                    unsafe extern "C" fn(),
                >(question_requested),
                data,
                None,
                0,
            );
        }
        let mounted = run(
            &api,
            &main,
            context,
            Finish::Boolean(api.g_file_mount_enclosing_volume_finish),
            |cancel, callback, data| {
                // SAFETY: run retains all references and drains completion before returning.
                unsafe {
                    (api.g_file_mount_enclosing_volume)(
                        file.pointer,
                        0,
                        operation.pointer,
                        cancel,
                        callback,
                        data,
                    );
                }
            },
        );
        context.check()?;
        if challenge.password.load(Ordering::Acquire) {
            return Err(Failure::new(
                ErrorCode::AuthenticationRequired,
                "This network location requires authentication. Connect through the desktop session, then retry.",
            ));
        }
        match mounted {
            Ok(_) => {}
            // Only G_IO_ERROR_ALREADY_MOUNTED is success, never a generic busy result.
            Err(error) if error.code == ErrorCode::Busy && error.os_code == Some(17) => {}
            Err(error) => return Err(error),
        }
        if target.kind != NodeKind::Share {
            return Ok(None);
        }
        context.check()?;
        // SAFETY: live mounted file. GVfs 1.54.4 may do synchronous mount-info I/O here;
        // no caller permit is released until it returns. Never claim a hard cancellation bound.
        let path = unsafe { (api.g_file_get_path)(file.pointer) };
        let value = if path.is_null() {
            Ok(None)
        } else {
            // SAFETY: get_path transfers ownership of a native byte string.
            let value = unsafe { bounded_bytes(path) }.map(Some);
            // SAFETY: release the transferred string even if validation or cancellation fails.
            unsafe {
                (api.g_free)(path.cast());
            }
            value
        };
        context.check()?;
        value
    }
}

#[cfg(test)]
mod tests;
