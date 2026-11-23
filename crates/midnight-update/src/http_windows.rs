//! WinHTTP transport: Windows' own HTTPS stack, certificate validation and proxy
//! settings. Redirects are turned off; the client follows them after its checks.
#![allow(unsafe_code)]
use crate::transport::{
    CONNECT_TIMEOUT, Head, RECEIVE_TIMEOUT, Request, Sink, Transport, TransportError, USER_AGENT,
    UrlPolicy, parse_content_range,
};
use std::{ffi::c_void, ptr};
use windows_sys::Win32::{
    Foundation::GetLastError,
    Networking::WinHttp::{
        ERROR_WINHTTP_CANNOT_CONNECT, ERROR_WINHTTP_CONNECTION_ERROR,
        ERROR_WINHTTP_NAME_NOT_RESOLVED, ERROR_WINHTTP_SECURE_FAILURE, ERROR_WINHTTP_TIMEOUT,
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ADDREQ_FLAG_ADD, WINHTTP_DISABLE_COOKIES,
        WINHTTP_FLAG_SECURE, WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2,
        WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3, WINHTTP_OPTION_DISABLE_FEATURE,
        WINHTTP_OPTION_REDIRECT_POLICY, WINHTTP_OPTION_REDIRECT_POLICY_NEVER,
        WINHTTP_OPTION_SECURE_PROTOCOLS, WINHTTP_QUERY_CUSTOM, WINHTTP_QUERY_FLAG_NUMBER,
        WINHTTP_QUERY_LOCATION, WINHTTP_QUERY_STATUS_CODE, WinHttpAddRequestHeaders,
        WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
        WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption,
        WinHttpSetTimeouts,
    },
};

/// The system transport. Each request opens its own WinHTTP session.
#[derive(Default)]
pub struct SystemTransport {
    policy: Option<UrlPolicy>,
}

impl SystemTransport {
    /// `policy` decides which addresses the transport connects to; the client checks
    /// the same policy, and this second check keeps a misused transport from plain HTTP.
    pub fn new(policy: UrlPolicy) -> Self {
        Self {
            policy: Some(policy),
        }
    }
}

/// Owns one WinHTTP handle.
struct Handle(*mut c_void);

impl Handle {
    fn open(raw: *mut c_void) -> Result<Self, TransportError> {
        if raw.is_null() {
            Err(last_error())
        } else {
            Ok(Self(raw))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful WinHTTP open and is closed once.
        unsafe { WinHttpCloseHandle(self.0) };
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn last_error() -> TransportError {
    // SAFETY: reads the calling thread's last-error value.
    let code = unsafe { GetLastError() };
    match code {
        ERROR_WINHTTP_TIMEOUT => TransportError::TimedOut,
        ERROR_WINHTTP_NAME_NOT_RESOLVED => TransportError::Unreachable("name not resolved".into()),
        ERROR_WINHTTP_CANNOT_CONNECT => TransportError::Unreachable("connection refused".into()),
        ERROR_WINHTTP_SECURE_FAILURE => {
            TransportError::Tls("the server's certificate is not trusted".into())
        }
        ERROR_WINHTTP_CONNECTION_ERROR => {
            TransportError::Protocol("the connection was reset".into())
        }
        other => TransportError::Protocol(format!("WinHTTP error {other}")),
    }
}

fn check(ok: i32) -> Result<(), TransportError> {
    if ok == 0 { Err(last_error()) } else { Ok(()) }
}

fn set_u32(handle: &Handle, option: u32, value: u32) -> bool {
    // SAFETY: the option takes a DWORD; the buffer is a live u32 of that size.
    unsafe {
        WinHttpSetOption(
            handle.0,
            option,
            ptr::from_ref(&value).cast(),
            size_of::<u32>() as u32,
        ) != 0
    }
}

fn status_code(request: &Handle) -> Result<u16, TransportError> {
    let mut status: u32 = 0;
    let mut length = size_of::<u32>() as u32;
    // SAFETY: a numeric query writes one u32 into the given buffer.
    check(unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            ptr::null(),
            ptr::from_mut(&mut status).cast(),
            &mut length,
            ptr::null_mut(),
        )
    })?;
    u16::try_from(status).map_err(|_| TransportError::Protocol("invalid status".into()))
}

/// A header's text, `None` when absent.
fn header(request: &Handle, level: u32, name: Option<&str>) -> Option<String> {
    let name = name.map(wide);
    let name_pointer = name.as_ref().map_or(ptr::null(), |name| name.as_ptr());
    let mut buffer = vec![0u16; 1024];
    let mut length = (buffer.len() * 2) as u32;
    // SAFETY: the buffer holds `length` bytes; WinHTTP writes at most that and reports
    // the written size in bytes without the terminator.
    let ok = unsafe {
        WinHttpQueryHeaders(
            request.0,
            level,
            name_pointer,
            buffer.as_mut_ptr().cast(),
            &mut length,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        return None;
    }
    buffer.truncate(length as usize / 2);
    String::from_utf16(&buffer).ok()
}

fn number(request: &Handle, name: &str) -> Option<u64> {
    header(request, WINHTTP_QUERY_CUSTOM, Some(name)).and_then(|text| text.trim().parse().ok())
}

impl Transport for SystemTransport {
    fn get(&self, request: &Request, sink: &mut dyn Sink) -> Result<Head, TransportError> {
        let policy = self.policy.clone().unwrap_or(UrlPolicy::GitHub);
        let target = policy
            .check(&request.url)
            .map_err(TransportError::Protocol)?;
        let agent = wide(USER_AGENT);
        // SAFETY: the agent string is NUL-terminated and outlives the call.
        let session = Handle::open(unsafe {
            WinHttpOpen(
                agent.as_ptr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                ptr::null(),
                ptr::null(),
                0,
            )
        })?;
        let connect_ms = CONNECT_TIMEOUT.as_millis() as i32;
        let receive_ms = RECEIVE_TIMEOUT.as_millis() as i32;
        // SAFETY: the session handle is live.
        check(unsafe {
            WinHttpSetTimeouts(session.0, connect_ms, connect_ms, receive_ms, receive_ms)
        })?;
        if target.secure
            && !set_u32(
                &session,
                WINHTTP_OPTION_SECURE_PROTOCOLS,
                WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3,
            )
        {
            // Windows 10 without TLS 1.3 in WinHTTP refuses the combined value.
            set_u32(
                &session,
                WINHTTP_OPTION_SECURE_PROTOCOLS,
                WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2,
            );
        }
        let host = wide(&target.host);
        // SAFETY: the host string is NUL-terminated and outlives the call.
        let connection =
            Handle::open(unsafe { WinHttpConnect(session.0, host.as_ptr(), target.port, 0) })?;
        let verb = wide("GET");
        let path = wide(&target.path);
        let flags = if target.secure {
            WINHTTP_FLAG_SECURE
        } else {
            0
        };
        // SAFETY: every string is NUL-terminated and outlives the call; a null accept
        // list sends no Accept header from WinHTTP itself.
        let handle = Handle::open(unsafe {
            WinHttpOpenRequest(
                connection.0,
                verb.as_ptr(),
                path.as_ptr(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                flags,
            )
        })?;
        if !set_u32(
            &handle,
            WINHTTP_OPTION_REDIRECT_POLICY,
            WINHTTP_OPTION_REDIRECT_POLICY_NEVER,
        ) || !set_u32(
            &handle,
            WINHTTP_OPTION_DISABLE_FEATURE,
            WINHTTP_DISABLE_COOKIES,
        ) {
            return Err(last_error());
        }
        let mut headers = format!("Accept: {}\r\n", request.accept);
        if request.accept.contains("github") {
            headers.push_str("X-GitHub-Api-Version: 2022-11-28\r\n");
        }
        if request.range_from > 0 {
            headers.push_str(&format!("Range: bytes={}-\r\n", request.range_from));
        }
        let headers = wide(&headers);
        // SAFETY: the header block is NUL-terminated (length -1 means "up to NUL").
        check(unsafe {
            WinHttpAddRequestHeaders(
                handle.0,
                headers.as_ptr(),
                u32::MAX,
                WINHTTP_ADDREQ_FLAG_ADD,
            )
        })?;
        // SAFETY: no additional headers or body are sent.
        check(unsafe { WinHttpSendRequest(handle.0, ptr::null(), 0, ptr::null(), 0, 0, 0) })?;
        // SAFETY: the request was sent on this handle.
        check(unsafe { WinHttpReceiveResponse(handle.0, ptr::null_mut()) })?;
        let status = status_code(&handle)?;
        let head = Head {
            status,
            location: header(&handle, WINHTTP_QUERY_LOCATION, None),
            content_length: number(&handle, "Content-Length"),
            range_start: header(&handle, WINHTTP_QUERY_CUSTOM, Some("Content-Range"))
                .and_then(|value| parse_content_range(&value)),
            retry_after: number(&handle, "Retry-After"),
            rate_remaining: number(&handle, "X-RateLimit-Remaining"),
            rate_reset: number(&handle, "X-RateLimit-Reset"),
        };
        if !head.is_success() {
            return Ok(head);
        }
        sink.begin(&head).map_err(TransportError::Sink)?;
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let mut read: u32 = 0;
            // SAFETY: the buffer holds `buffer.len()` bytes for WinHTTP to fill.
            check(unsafe {
                WinHttpReadData(
                    handle.0,
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    &mut read,
                )
            })?;
            if read == 0 {
                break;
            }
            sink.write(&buffer[..read as usize])
                .map_err(TransportError::Sink)?;
        }
        Ok(head)
    }
}
