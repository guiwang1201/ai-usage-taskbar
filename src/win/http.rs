//! HTTPS GET through WinHTTP: no extra code in the binary, and it follows the system proxy.

use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::Networking::WinHttp::*;

use super::wide;
use crate::providers::{Request, Response};

const MAX_BODY: usize = 1 << 20;

struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

fn failed(step: &str) -> String {
    format!("{step} failed ({})", unsafe { GetLastError() })
}

pub fn get(request: &Request) -> Result<Response, String> {
    let agent = wide(&request.user_agent);
    let host = wide(request.host);
    let path = wide(request.path);
    let verb = wide("GET");
    let headers: String = request.headers.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
    let headers = wide(&headers);

    unsafe {
        let session = Handle(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0));
        if session.0.is_null() {
            return Err(failed("WinHttpOpen"));
        }
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 15_000, 15_000);

        let connection = Handle(WinHttpConnect(session.0, host.as_ptr(), 443, 0));
        if connection.0.is_null() {
            return Err(failed("WinHttpConnect"));
        }
        let req = Handle(WinHttpOpenRequest(
            connection.0,
            verb.as_ptr(),
            path.as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ));
        if req.0.is_null() {
            return Err(failed("WinHttpOpenRequest"));
        }
        // Never follow a redirect: the request carries a bearer token, and following one
        // would hand that token to whatever host the redirect names. Cookies are not
        // needed either. If this cannot be set, nothing is sent.
        let disabled: u32 = WINHTTP_DISABLE_REDIRECTS | WINHTTP_DISABLE_COOKIES;
        if WinHttpSetOption(
            req.0,
            WINHTTP_OPTION_DISABLE_FEATURE,
            &disabled as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        ) == 0
        {
            return Err(failed("WinHttpSetOption"));
        }
        if WinHttpAddRequestHeaders(req.0, headers.as_ptr(), u32::MAX, WINHTTP_ADDREQ_FLAG_ADD) == 0 {
            return Err(failed("WinHttpAddRequestHeaders"));
        }
        if WinHttpSendRequest(req.0, null(), 0, null(), 0, 0, 0) == 0 {
            return Err(failed("WinHttpSendRequest"));
        }
        if WinHttpReceiveResponse(req.0, null_mut()) == 0 {
            return Err(failed("WinHttpReceiveResponse"));
        }

        let mut status: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        if WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            &mut status as *mut u32 as *mut c_void,
            &mut size,
            null_mut(),
        ) == 0
        {
            return Err(failed("WinHttpQueryHeaders"));
        }
        // Fails when the header is missing or is a date rather than a number of seconds.
        let mut seconds: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let retry_after = (WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_RETRY_AFTER | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            &mut seconds as *mut u32 as *mut c_void,
            &mut size,
            null_mut(),
        ) != 0)
            .then_some(seconds);

        let mut body = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let mut read: u32 = 0;
            if WinHttpReadData(req.0, chunk.as_mut_ptr() as *mut c_void, chunk.len() as u32, &mut read) == 0 {
                return Err(failed("WinHttpReadData"));
            }
            if read == 0 || body.len() >= MAX_BODY {
                break;
            }
            body.extend_from_slice(&chunk[..read as usize]);
        }
        Ok(Response { status: status as u16, body: String::from_utf8_lossy(&body).into_owned(), retry_after })
    }
}
