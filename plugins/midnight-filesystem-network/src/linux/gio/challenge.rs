//! One explicit submission can answer one password prompt on the selected mount.
use super::{Object, OwnedObject, Symbols, unavailable};
use crate::{
    adapter::Context,
    api::{ErrorCode, Failure, MAX_NETWORK_CREDENTIAL_BYTES, NetworkCredentials},
};
use std::{
    ffi::{c_char, c_int},
    ptr,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};
use zeroize::Zeroizing;

const NEED_PASSWORD: u32 = 1;
const NEED_USERNAME: u32 = 2;
const NEED_DOMAIN: u32 = 4;
const SAVING_SUPPORTED: u32 = 8;
const ANONYMOUS_SUPPORTED: u32 = 16;
const SUPPORTED_PASSWORD_FLAGS: u32 =
    NEED_PASSWORD | NEED_USERNAME | NEED_DOMAIN | SAVING_SUPPORTED | ANONYMOUS_SUPPORTED;
const PASSWORD_SAVE_NEVER: c_int = 0;
const HANDLED: c_int = 0;
const ABORTED: c_int = 1;

struct CredentialFields {
    username: Zeroizing<Vec<u8>>,
    domain: Zeroizing<Vec<u8>>,
    password: Zeroizing<Vec<u8>>,
}

impl CredentialFields {
    fn new(credentials: &NetworkCredentials) -> Result<Self, Failure> {
        fn field(bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>, Failure> {
            if bytes.len() > MAX_NETWORK_CREDENTIAL_BYTES
                || bytes.contains(&0)
                || std::str::from_utf8(bytes).is_err()
            {
                return Err(Failure::new(
                    ErrorCode::Invalid,
                    "Invalid network sign-in fields.",
                ));
            }
            let mut value = Zeroizing::new(Vec::new());
            value.try_reserve_exact(bytes.len() + 1).map_err(|_| {
                Failure::new(
                    ErrorCode::ResourceLimit,
                    "Could not allocate sign-in fields.",
                )
            })?;
            value.extend_from_slice(bytes);
            value.push(0);
            Ok(value)
        }
        if credentials.username.is_empty() {
            return Err(Failure::new(
                ErrorCode::Invalid,
                "A sign-in username is required.",
            ));
        }
        Ok(Self {
            username: field(credentials.username.as_bytes())?,
            domain: field(credentials.domain.as_bytes())?,
            password: field(credentials.password())?,
        })
    }
}

const NOT_ASKED: u8 = 0;
const CREDENTIALS_SENT: u8 = 1;
const SIGN_IN_REQUIRED: u8 = 2;
const UNSUPPORTED_CHALLENGE: u8 = 3;

struct Challenge<'a> {
    api: &'a Symbols,
    context: &'a Context<'a>,
    credentials: Option<CredentialFields>,
    status: AtomicU8,
}

pub(super) struct MountChallenge<'a> {
    operation: OwnedObject,
    state: Box<Challenge<'a>>,
    handlers: [usize; 2],
}

impl<'a> MountChallenge<'a> {
    pub(super) fn new(
        api: &'a Arc<Symbols>,
        context: &'a Context<'a>,
        credentials: Option<&NetworkCredentials>,
    ) -> Result<Self, Failure> {
        let credentials = credentials.map(CredentialFields::new).transpose()?;
        let mut state = Box::new(Challenge {
            api,
            context,
            credentials,
            status: AtomicU8::new(NOT_ASKED),
        });
        // SAFETY: a fresh GMountOperation is an owned GObject.
        let operation = OwnedObject::new(api, unsafe { (api.g_mount_operation_new)() })?;
        // SAFETY: the signal signatures match GMountOperation. The heap allocation
        // stays live until Drop disconnects both callbacks from this operation.
        let handlers = unsafe {
            (api.g_mount_operation_set_password_save)(operation.pointer, PASSWORD_SAVE_NEVER);
            let data = (&mut *state as *mut Challenge<'_>).cast();
            [
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
                ),
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
                ),
            ]
        };
        let challenge = Self {
            operation,
            state,
            handlers,
        };
        if handlers.contains(&0) {
            return Err(unavailable());
        }
        Ok(challenge)
    }

    pub(super) fn pointer(&self) -> Object {
        self.operation.pointer
    }

    pub(super) fn check(&self) -> Result<(), Failure> {
        self.state.context.check()?;
        match self.state.status.load(Ordering::Acquire) {
            SIGN_IN_REQUIRED => Err(Failure::new(
                ErrorCode::AuthenticationRequired,
                "This network location requires sign-in.",
            )),
            UNSUPPORTED_CHALLENGE => Err(Failure::new(
                ErrorCode::Unsupported,
                "This network location requested unsupported sign-in information.",
            )),
            _ => Ok(()),
        }
    }
}

impl Drop for MountChallenge<'_> {
    fn drop(&mut self) {
        // SAFETY: run drains its operation before this guard drops. The operation and
        // state are still live. Disconnect before freeing state even if GIO kept a ref.
        unsafe {
            for handler in self.handlers {
                if handler != 0 {
                    (self.state.api.g_signal_handler_disconnect)(self.operation.pointer, handler);
                }
            }
            (self.state.api.g_mount_operation_set_password)(self.operation.pointer, ptr::null());
        }
    }
}

unsafe extern "C" fn password_requested(
    operation: Object,
    _message: *const c_char,
    _user: *const c_char,
    _domain: *const c_char,
    flags: u32,
    data: Object,
) {
    // SAFETY: MountChallenge retains this state until it disconnects the handlers.
    let challenge = unsafe { &*data.cast::<Challenge<'_>>() };
    let api = challenge.api;
    if challenge.context.check().is_ok() {
        if flags & NEED_PASSWORD == 0 || (flags & !SUPPORTED_PASSWORD_FLAGS) != 0 {
            challenge
                .status
                .store(UNSUPPORTED_CHALLENGE, Ordering::Release);
        } else if let Some(fields) = &challenge.credentials
            && challenge
                .status
                .compare_exchange(
                    NOT_ASKED,
                    CREDENTIALS_SENT,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
        {
            // SAFETY: the operation is live and the fields are NUL-terminated UTF-8.
            // GIO copies the strings. An empty domain leaves GVfs's default in force.
            unsafe {
                if flags & NEED_USERNAME != 0 {
                    (api.g_mount_operation_set_username)(
                        operation,
                        fields.username.as_ptr().cast(),
                    );
                }
                if flags & NEED_DOMAIN != 0 {
                    (api.g_mount_operation_set_domain)(operation, fields.domain.as_ptr().cast());
                }
                (api.g_mount_operation_set_password)(operation, fields.password.as_ptr().cast());
                (api.g_mount_operation_reply)(operation, HANDLED);
            }
            return;
        } else {
            let _ = challenge
                .status
                .try_update(Ordering::AcqRel, Ordering::Acquire, |status| {
                    (status != UNSUPPORTED_CHALLENGE).then_some(SIGN_IN_REQUIRED)
                });
        }
    }
    // SAFETY: the operation is live. A subsequent prompt gets no credential reply.
    unsafe {
        (api.g_mount_operation_set_password)(operation, ptr::null());
        (api.g_mount_operation_reply)(operation, ABORTED);
    }
}

unsafe extern "C" fn question_requested(
    operation: Object,
    _message: *const c_char,
    _choices: *const *const c_char,
    data: Object,
) {
    // SAFETY: same owned activation state as password_requested.
    let challenge = unsafe { &*data.cast::<Challenge<'_>>() };
    challenge
        .status
        .store(UNSUPPORTED_CHALLENGE, Ordering::Release);
    // SAFETY: reject interaction rather than presenting an implicit question.
    unsafe {
        (challenge.api.g_mount_operation_reply)(operation, ABORTED);
    }
}
