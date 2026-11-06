use crate::{api::abi, callbacks::Bridge, loaded};
pub(crate) struct PendingReply {
    pub id: u64,
    pub payload: Vec<u8>,
    pub buffer: abi::Buffer,
}
impl PendingReply {
    pub fn deliver(
        &self,
        bridge: &Bridge,
        submit: impl FnOnce(&abi::Request) -> abi::Status,
    ) -> bool {
        let request = abi::Request {
            id: self.id,
            session: 1,
            generation: bridge.shared.generation,
            kind: abi::REQUEST_SERVICE_REPLY,
            flags: 0,
            payload: loaded::slice(&self.payload),
            content: self.buffer,
        };
        let status = submit(&request);
        if status == abi::QUEUE_FULL {
            return false;
        }
        bridge.release(self.buffer.id);
        bridge
            .services
            .fetch_sub(1, std::sync::atomic::Ordering::Release);
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queue_full_service_reply_keeps_buffer_and_admission_until_retry_succeeds() {
        let (bridge, _) = Bridge::new(crate::session::tests::shared());
        bridge
            .services
            .store(1, std::sync::atomic::Ordering::Release);
        let buffer = bridge.allocate(vec![1, 2, 3]).unwrap();
        let reply = PendingReply {
            id: 5,
            payload: Vec::new(),
            buffer,
        };
        assert!(!reply.deliver(&bridge, |_| abi::QUEUE_FULL));
        assert!(!bridge.empty());
        assert_eq!(
            unsafe { loaded::bytes(buffer.bytes, 3).unwrap() },
            &[1, 2, 3]
        );
        assert!(reply.deliver(&bridge, |_| abi::OK));
        assert!(bridge.empty());
    }
}
