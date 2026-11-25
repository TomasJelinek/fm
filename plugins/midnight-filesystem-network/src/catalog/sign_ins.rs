//! Sign-ins remembered for the rest of the session, per server (`Adapter::sign_in_scope`).
//!
//! Credentials submitted to the sign-in dialog answer the one password challenge of their
//! retry. Once such a sign-in succeeds, a copy stays in this instance's memory for its
//! server until the provider closes with the application: a later explicit activation of a
//! server or share there offers it to that activation's password challenge, once. A server
//! that rejects it makes the provider forget it, so that the dialog asks again. Nothing is
//! written anywhere, and other servers, listings and discovery never receive it.
use super::state::lock;
use crate::{
    adapter::{Adapter, Context, Target},
    api::{ActivationTarget, ErrorCode, Failure, NetworkCredentials},
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

/// At most this many servers keep a sign-in; the one remembered longest ago gives way.
const MAX_SERVERS: usize = 32;

#[derive(Default)]
pub(super) struct SignIns {
    remembered: Mutex<VecDeque<(Vec<u8>, Arc<NetworkCredentials>)>>,
}

impl SignIns {
    /// One explicit activation of `target`: a sign-in with the `submitted` credentials,
    /// else one with the credentials remembered for its server, else a plain activation.
    /// Returns the adapter's target and whether credentials were offered.
    pub(super) fn activate<A: Adapter>(
        &self,
        target: &Target,
        submitted: Option<&NetworkCredentials>,
        context: &Context<'_>,
    ) -> Result<(ActivationTarget, bool), Failure> {
        let scope = A::sign_in_scope(target);
        if let Some(credentials) = submitted {
            let activation = A::sign_in(target, credentials, context)?;
            if let Some(scope) = scope {
                self.remember(scope, credentials);
            }
            return Ok((activation, true));
        }
        let Some((scope, remembered)) =
            scope.and_then(|scope| self.get(&scope).map(|credentials| (scope, credentials)))
        else {
            return A::activate(target, context).map(|activation| (activation, false));
        };
        match A::sign_in(target, &remembered, context) {
            Ok(activation) => Ok((activation, true)),
            Err(error) => {
                if error.code == ErrorCode::AuthenticationRequired {
                    self.forget(&scope, &remembered);
                }
                Err(error)
            }
        }
    }

    fn get(&self, scope: &[u8]) -> Option<Arc<NetworkCredentials>> {
        lock(&self.remembered)
            .iter()
            .find(|(known, _)| known.as_slice() == scope)
            .map(|(_, credentials)| credentials.clone())
    }

    fn remember(&self, scope: Vec<u8>, credentials: &NetworkCredentials) {
        // The copy's password lives in a buffer that is wiped when the last holder drops it.
        let copy = Arc::new(NetworkCredentials::new(
            credentials.username.clone(),
            credentials.domain.clone(),
            credentials.password().to_vec(),
        ));
        let mut remembered = lock(&self.remembered);
        remembered.retain(|(known, _)| *known != scope);
        if remembered.len() >= MAX_SERVERS {
            remembered.pop_front();
        }
        remembered.push_back((scope, copy));
    }

    /// Forget the rejected `used` credentials, unless a newer sign-in has replaced them.
    fn forget(&self, scope: &[u8], used: &Arc<NetworkCredentials>) {
        lock(&self.remembered)
            .retain(|(known, credentials)| known != scope || !Arc::ptr_eq(credentials, used));
    }
}

#[cfg(test)]
mod tests;
