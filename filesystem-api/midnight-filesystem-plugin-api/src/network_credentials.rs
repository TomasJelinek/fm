use crate::WireError;
use zeroize::{Zeroize, Zeroizing};

/// Maximum length of each SignIn field, measured in UTF-8 bytes.
pub const MAX_NETWORK_CREDENTIAL_BYTES: usize = 4096;

/// One-use credentials for activating an explicitly chosen network location.
/// UTF-8 fields may not contain NUL; the username must be nonempty. The codec
/// checks these constraints before sending or accepting a request. All fields
/// are wiped on drop and omitted from debug output.
#[derive(Clone, PartialEq, Eq)]
pub struct NetworkCredentials {
    pub username: String,
    pub domain: String,
    password: Zeroizing<Vec<u8>>,
}

impl NetworkCredentials {
    pub fn new(username: String, domain: String, password: Vec<u8>) -> Self {
        Self {
            username,
            domain,
            password: Zeroizing::new(password),
        }
    }

    pub fn password(&self) -> &[u8] {
        &self.password
    }

    pub(crate) fn validate(&self) -> Result<(), WireError> {
        Self::validate_fields(
            self.username.as_bytes(),
            self.domain.as_bytes(),
            self.password(),
        )
        .map(|_| ())
    }

    pub(crate) fn validate_fields<'a>(
        username: &'a [u8],
        domain: &'a [u8],
        password: &'a [u8],
    ) -> Result<[&'a str; 3], WireError> {
        fn field(bytes: &[u8]) -> Result<&str, WireError> {
            if bytes.len() > MAX_NETWORK_CREDENTIAL_BYTES {
                return Err(WireError::Limit);
            }
            if bytes.contains(&0) {
                return Err(WireError::Malformed);
            }
            std::str::from_utf8(bytes).map_err(|_| WireError::Malformed)
        }
        if username.is_empty() {
            return Err(WireError::Malformed);
        }
        Ok([field(username)?, field(domain)?, field(password)?])
    }

    pub(crate) fn copy_validated(
        [username, domain, password]: [&str; 3],
    ) -> Result<Self, WireError> {
        let mut credentials = Self::new(String::new(), String::new(), Vec::new());
        credentials
            .username
            .try_reserve_exact(username.len())
            .map_err(|_| WireError::Limit)?;
        credentials
            .domain
            .try_reserve_exact(domain.len())
            .map_err(|_| WireError::Limit)?;
        credentials
            .password
            .try_reserve_exact(password.len())
            .map_err(|_| WireError::Limit)?;
        credentials.username.push_str(username);
        credentials.domain.push_str(domain);
        credentials.password.extend_from_slice(password.as_bytes());
        Ok(credentials)
    }
}

impl Drop for NetworkCredentials {
    fn drop(&mut self) {
        self.username.zeroize();
        self.domain.zeroize();
    }
}

impl std::fmt::Debug for NetworkCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("NetworkCredentials(<redacted>)")
    }
}
