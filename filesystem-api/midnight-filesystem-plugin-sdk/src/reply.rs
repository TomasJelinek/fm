use crate::{Failure, Response, api::ErrorCode};

pub(crate) type Completion = Box<dyn FnOnce(bool) -> Result<(), Failure> + Send>;

/// A response owns newly created resources until its encoded buffer is published.
/// The completion runs exactly once: `true` commits publication, `false` rolls back.
/// Cleanup must ignore request cancellation and must not call back into this instance.
pub struct Reply {
    pub(crate) response: Response,
    pub(crate) completion: Option<Completion>,
}
impl Reply {
    /// Resource-free responses need no publication transaction.
    pub fn stateless(response: Response) -> Result<Self, Failure> {
        if matches!(
            response,
            Response::Resolved { .. } | Response::Opened { .. } | Response::Page { .. }
        ) {
            return Err(Failure::new(
                ErrorCode::Invalid,
                "Resource response requires publication ownership.",
            ));
        }
        Ok(Self {
            response,
            completion: None,
        })
    }
    pub fn owned(
        response: Response,
        completion: impl FnOnce(bool) -> Result<(), Failure> + Send + 'static,
    ) -> Self {
        Self {
            response,
            completion: Some(Box::new(completion)),
        }
    }
    pub(crate) fn into_parts(mut self) -> (Response, Option<Completion>) {
        (
            std::mem::replace(&mut self.response, Response::Unit),
            self.completion.take(),
        )
    }
}
impl Drop for Reply {
    fn drop(&mut self) {
        if let Some(completion) = self.completion.take() {
            // A handler can unwind after constructing a reply; never double-unwind.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| completion(false)));
        }
    }
}
